//! Zero-`chrono` time utilities: epoch timestamps, RFC 3339 formatting, HTTP/cookie date
//! parsing, and local-timezone "next 8:00 AM" calculation.

use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

/// Current wall-clock time in milliseconds since `1970-01-01T00:00:00Z`.
pub fn now_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// Current wall-clock time in seconds since `1970-01-01T00:00:00Z`.
pub fn now_secs() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Howard Hinnant's `civil_from_days`: converts days since `1970-01-01` to `(year, month, day)`.
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
}

/// Howard Hinnant's `days_from_civil`: converts `(year, month, day)` to days since `1970-01-01`.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = year - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64; // [0, 399]
    let m = month as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + (day as u64) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + (doe as i64) - 719_468
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn split_epoch_secs(epoch_secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = epoch_secs.div_euclid(86_400);
    let secs_of_day = epoch_secs.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    (year, month, day, hour, minute, second)
}

/// Formats `epoch_ms` as UTC RFC 3339 with millisecond precision: `"YYYY-MM-DDTHH:MM:SS.mmmZ"`.
pub fn format_rfc3339_millis(epoch_ms: i64) -> String {
    let epoch_secs = epoch_ms.div_euclid(1000);
    let millis = epoch_ms.rem_euclid(1000) as u32;
    let (year, month, day, hour, minute, second) = split_epoch_secs(epoch_secs);
    let mut out = String::with_capacity(24);
    let _ = write!(
        out,
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    );
    out
}

/// Formats `epoch_secs` as UTC RFC 3339 with second precision: `"YYYY-MM-DDTHH:MM:SSZ"`.
pub fn format_rfc3339_secs(epoch_secs: i64) -> String {
    let (year, month, day, hour, minute, second) = split_epoch_secs(epoch_secs);
    let mut out = String::with_capacity(20);
    let _ = write!(
        out,
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    );
    out
}

/// Formats `epoch_ms` as UTC RFC 3339 with second precision (flooring sub-second milliseconds).
pub fn format_rfc3339_secs_from_ms(epoch_ms: i64) -> String {
    format_rfc3339_secs(epoch_ms.div_euclid(1000))
}

fn parse_month_abbr(s: &str) -> Option<u32> {
    if s.len() < 3 {
        return None;
    }
    let b = s.as_bytes();
    let k = [
        b[0].to_ascii_lowercase(),
        b[1].to_ascii_lowercase(),
        b[2].to_ascii_lowercase(),
    ];
    match &k {
        b"jan" => Some(1),
        b"feb" => Some(2),
        b"mar" => Some(3),
        b"apr" => Some(4),
        b"may" => Some(5),
        b"jun" => Some(6),
        b"jul" => Some(7),
        b"aug" => Some(8),
        b"sep" => Some(9),
        b"oct" => Some(10),
        b"nov" => Some(11),
        b"dec" => Some(12),
        _ => None,
    }
}

fn parse_ascii_uint(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>().ok()
}

fn parse_hms(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.split(':');
    let h = parse_ascii_uint(parts.next()?)?;
    let m = parse_ascii_uint(parts.next()?)?;
    let sec_raw = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    // Strip optional fractional seconds (e.g. "00.123")
    let sec_str = sec_raw.split('.').next()?;
    let sec = parse_ascii_uint(sec_str)?;
    if h > 23 || m > 59 || sec > 60 {
        return None;
    }
    Some((h, m, sec.min(59)))
}

fn compose_epoch_secs(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    tz_offset_secs: i64,
) -> Option<i64> {
    if !(1..=12).contains(&month) {
        return None;
    }
    let max_day = days_in_month(year, month);
    if day < 1 || day > max_day {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let secs = days
        .checked_mul(86_400)?
        .checked_add((hour as i64) * 3600 + (minute as i64) * 60 + (second as i64))?
        .checked_sub(tz_offset_secs)?;
    Some(secs)
}

fn expand_two_digit_year(y: i64) -> i64 {
    if (0..=69).contains(&y) {
        2000 + y
    } else if (70..=99).contains(&y) {
        1900 + y
    } else {
        y
    }
}

fn parse_tz_offset(tz: &str) -> Option<i64> {
    let t = tz.trim();
    if t.is_empty()
        || t.eq_ignore_ascii_case("gmt")
        || t.eq_ignore_ascii_case("utc")
        || t.eq_ignore_ascii_case("ut")
        || t.eq_ignore_ascii_case("z")
    {
        return Some(0);
    }
    let bytes = t.as_bytes();
    if (bytes[0] == b'+' || bytes[0] == b'-') && (bytes.len() == 5 || bytes.len() == 6) {
        let sign: i64 = if bytes[0] == b'-' { -1 } else { 1 };
        let (hh_s, mm_s) = if bytes.len() == 5 {
            (&t[1..3], &t[3..5])
        } else if bytes[3] == b':' {
            (&t[1..3], &t[4..6])
        } else {
            return None;
        };
        let hh = parse_ascii_uint(hh_s)?;
        let mm = parse_ascii_uint(mm_s)?;
        if hh > 23 || mm > 59 {
            return None;
        }
        return Some(sign * ((hh as i64) * 3600 + (mm as i64) * 60));
    }
    // Common RFC 2822 North American timezone abbreviations
    match t.to_ascii_uppercase().as_str() {
        "EST" => Some(-5 * 3600),
        "EDT" => Some(-4 * 3600),
        "CST" => Some(-6 * 3600),
        "CDT" => Some(-5 * 3600),
        "MST" => Some(-7 * 3600),
        "MDT" => Some(-6 * 3600),
        "PST" => Some(-8 * 3600),
        "PDT" => Some(-7 * 3600),
        _ => None,
    }
}

/// Parses an RFC 3339 date-time string into Unix epoch seconds.
fn parse_rfc3339_epoch_secs(s: &str) -> Option<i64> {
    // Expected: YYYY-MM-DD[T|t| ]HH:MM:SS[.frac](Z|z|+HH:MM|-HH:MM|+HHMM|-HHMM)
    let bytes = s.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if !matches!(bytes[10], b'T' | b't' | b' ') {
        return None;
    }
    let year = parse_ascii_uint(&s[0..4])? as i64;
    let month = parse_ascii_uint(&s[5..7])?;
    let day = parse_ascii_uint(&s[8..10])?;

    let rest = &s[11..];
    // Find timezone marker after HH:MM:SS (at index >= 8)
    let tz_pos = rest
        .char_indices()
        .skip(8)
        .find(|&(_, ch)| matches!(ch, 'Z' | 'z' | '+' | '-'))
        .map(|(i, _)| i)?;
    let (time_part, tz_part) = rest.split_at(tz_pos);
    let (hour, minute, second) = parse_hms(time_part)?;
    let tz_offset = parse_tz_offset(tz_part)?;
    compose_epoch_secs(year, month, day, hour, minute, second, tz_offset)
}

/// Parses an HTTP, cookie, or ISO/RFC 3339 date string into Unix epoch seconds.
///
/// Supported formats:
/// - RFC 3339: `"2025-10-21T07:28:00Z"`, `"2025-10-21T07:28:00.123+07:00"`
/// - RFC 2822 / IMF-fixdate: `"Wed, 21 Oct 2015 07:28:00 GMT"`, `"Wed, 21 Oct 2015 07:28:00 +0700"`
/// - RFC 850 / cookie format: `"Wed, 21-Oct-2015 07:28:00 GMT"`, `"Wed, 21-Oct-25 07:28:00 GMT"`
/// - ANSI C `asctime()`: `"Wed Oct 21 07:28:00 2015"`
pub fn parse_http_date_epoch_secs(s: &str) -> Option<i64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. RFC 3339 ("YYYY-MM-DDT...")
    if let Some(secs) = parse_rfc3339_epoch_secs(trimmed) {
        return Some(secs);
    }

    // Strip optional leading weekday + comma (e.g., "Wed, " or "Sunday, ")
    let after_comma = if let Some((_, rest)) = trimmed.split_once(',') {
        rest.trim()
    } else {
        trimmed
    };

    let tokens: Vec<&str> = after_comma.split_whitespace().collect();

    // 2. RFC 850 / dashed cookie format: "21-Oct-2015 07:28:00 GMT" (or 2-digit year "21-Oct-25")
    if tokens.len() == 3 && tokens[0].contains('-') {
        let mut dmy = tokens[0].split('-');
        let day = parse_ascii_uint(dmy.next()?)?;
        let month = parse_month_abbr(dmy.next()?)?;
        let raw_year = parse_ascii_uint(dmy.next()?)? as i64;
        if dmy.next().is_some() {
            return None;
        }
        let year = if tokens[0].rsplit('-').next().is_some_and(|y| y.len() == 2) {
            expand_two_digit_year(raw_year)
        } else {
            raw_year
        };
        let (hour, minute, second) = parse_hms(tokens[1])?;
        let tz_offset = parse_tz_offset(tokens[2])?;
        return compose_epoch_secs(year, month, day, hour, minute, second, tz_offset);
    }

    // 3. RFC 2822 / IMF-fixdate: "21 Oct 2015 07:28:00 GMT" (or "+0000", optional tz)
    if tokens.len() == 4 || tokens.len() == 5 {
        if let (Some(day), Some(month), Some(raw_year)) = (
            parse_ascii_uint(tokens[0]),
            parse_month_abbr(tokens[1]),
            parse_ascii_uint(tokens[2]),
        ) {
            let year = if tokens[2].len() == 2 {
                expand_two_digit_year(raw_year as i64)
            } else {
                raw_year as i64
            };
            let (hour, minute, second) = parse_hms(tokens[3])?;
            let tz_offset = if tokens.len() == 5 {
                parse_tz_offset(tokens[4])?
            } else {
                0
            };
            return compose_epoch_secs(year, month, day, hour, minute, second, tz_offset);
        }
    }

    // 4. ANSI C asctime(): "Wed Oct 21 07:28:00 2015" (no comma)
    let raw_tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if raw_tokens.len() == 5 {
        if let (Some(month), Some(day), Some((hour, minute, second)), Some(year)) = (
            parse_month_abbr(raw_tokens[1]),
            parse_ascii_uint(raw_tokens[2]),
            parse_hms(raw_tokens[3]),
            parse_ascii_uint(raw_tokens[4]),
        ) {
            return compose_epoch_secs(year as i64, month, day, hour, minute, second, 0);
        }
    }

    None
}

/// Computes the next local 08:00:00 (today if local hour < 8, else tomorrow) as epoch ms.
///
/// Uses `libc::localtime_r` + `libc::mktime` on Unix so phone/host local timezone and DST
/// transitions are respected accurately, with a UTC fallback on non-Unix targets.
pub fn next_local_8am_epoch_ms(now_ms: i64) -> i64 {
    #[cfg(unix)]
    {
        if let Some(ms) = next_local_8am_unix(now_ms) {
            return ms;
        }
    }
    next_utc_8am_epoch_ms(now_ms)
}

#[cfg(unix)]
fn next_local_8am_unix(now_ms: i64) -> Option<i64> {
    // `time_t` is 32 bits on 32-bit Android (until 2038): a time that does not fit must fall back to the UTC
    // computation, not wrap around into 1901..1970.
    let now_secs = libc::time_t::try_from(now_ms.div_euclid(1000)).ok()?;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let res = unsafe { libc::localtime_r(&now_secs, &mut tm) };
    if res.is_null() {
        return None;
    }
    if tm.tm_hour >= 8 {
        tm.tm_mday += 1;
    }
    tm.tm_hour = 8;
    tm.tm_min = 0;
    tm.tm_sec = 0;
    // Let mktime determine whether DST is in effect at the target local time.
    tm.tm_isdst = -1;
    let target_secs = unsafe { libc::mktime(&mut tm) };
    if target_secs == -1 {
        return None;
    }
    Some((target_secs as i64) * 1000)
}

fn next_utc_8am_epoch_ms(now_ms: i64) -> i64 {
    let now_secs = now_ms.div_euclid(1000);
    let secs_of_day = now_secs.rem_euclid(86_400);
    let day_start = now_secs - secs_of_day;
    let eight_am = 8 * 3600;
    let target_secs = if secs_of_day < eight_am {
        day_start + eight_am
    } else {
        day_start + 86_400 + eight_am
    };
    target_secs * 1000
}

/// Seconds from `now_ms` until the next local 08:00:00 (at least 1 second).
pub fn seconds_until_next_local_8am(now_ms: i64) -> i64 {
    let target_ms = next_local_8am_epoch_ms(now_ms);
    ((target_ms - now_ms) / 1000).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_formatting_epoch_leap_years_and_2026() {
        assert_eq!(format_rfc3339_millis(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_rfc3339_secs(0), "1970-01-01T00:00:00Z");
        // Leap day 2000-02-29T23:59:59.999Z
        let leap_2000 = days_from_civil(2000, 2, 29) * 86_400_000 + 86_399_999;
        assert_eq!(
            format_rfc3339_millis(leap_2000),
            "2000-02-29T23:59:59.999Z"
        );
        // Leap day 2024-02-29T12:34:56.007Z
        let leap_2024 =
            days_from_civil(2024, 2, 29) * 86_400_000 + (12 * 3600 + 34 * 60 + 56) * 1000 + 7;
        assert_eq!(
            format_rfc3339_millis(leap_2024),
            "2024-02-29T12:34:56.007Z"
        );
        // 2026-01-01T12:00:00.000Z = 1767268800000 ms
        assert_eq!(
            format_rfc3339_millis(1_767_268_800_000),
            "2026-01-01T12:00:00.000Z"
        );
        assert_eq!(
            format_rfc3339_secs_from_ms(1_767_268_800_987),
            "2026-01-01T12:00:00Z"
        );
        // Negative epoch (-1 ms -> 1969-12-31T23:59:59.999Z)
        assert_eq!(format_rfc3339_millis(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn parse_http_and_cookie_dates() {
        // RFC 2822 / IMF-fixdate
        assert_eq!(
            parse_http_date_epoch_secs("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(1_445_412_480)
        );
        assert_eq!(
            parse_http_date_epoch_secs("Thu, 01 Jan 1970 00:00:00 GMT"),
            Some(0)
        );
        assert_eq!(
            parse_http_date_epoch_secs("Thu, 01 Jan 2026 12:00:06 GMT"),
            Some(1_767_268_806)
        );
        // RFC 2822 with numeric timezone offset
        assert_eq!(
            parse_http_date_epoch_secs("Wed, 21 Oct 2015 14:28:00 +0700"),
            Some(1_445_412_480)
        );
        // RFC 850 / cookie format (4-digit & 2-digit years)
        assert_eq!(
            parse_http_date_epoch_secs("Wed, 21-Oct-2015 07:28:00 GMT"),
            Some(1_445_412_480)
        );
        assert_eq!(
            parse_http_date_epoch_secs("Tue, 21-Oct-25 07:28:00 GMT"),
            parse_http_date_epoch_secs("2025-10-21T07:28:00Z")
        );
        assert_eq!(
            parse_http_date_epoch_secs("Sunday, 06-Nov-94 08:49:37 GMT"),
            parse_http_date_epoch_secs("1994-11-06T08:49:37Z")
        );
        // RFC 3339 (Z, fractional seconds, +/-HH:MM)
        assert_eq!(
            parse_http_date_epoch_secs("2015-10-21T07:28:00Z"),
            Some(1_445_412_480)
        );
        assert_eq!(
            parse_http_date_epoch_secs("2015-10-21T07:28:00.999Z"),
            Some(1_445_412_480)
        );
        assert_eq!(
            parse_http_date_epoch_secs("2015-10-21T14:28:00+07:00"),
            Some(1_445_412_480)
        );
        assert_eq!(
            parse_http_date_epoch_secs("2015-10-21T02:28:00-05:00"),
            Some(1_445_412_480)
        );
        // ANSI C asctime()
        assert_eq!(
            parse_http_date_epoch_secs("Wed Oct 21 07:28:00 2015"),
            Some(1_445_412_480)
        );
        assert_eq!(
            parse_http_date_epoch_secs("Sun Nov  6 08:49:37 1994"),
            parse_http_date_epoch_secs("1994-11-06T08:49:37Z")
        );
        // Invalid inputs
        assert_eq!(parse_http_date_epoch_secs(""), None);
        assert_eq!(parse_http_date_epoch_secs("not a date"), None);
        assert_eq!(parse_http_date_epoch_secs("2025-02-29T00:00:00Z"), None);
        assert_eq!(parse_http_date_epoch_secs("2025-13-01T00:00:00Z"), None);
    }

    #[test]
    fn next_local_8am_is_strictly_in_future_within_24h() {
        let now = now_ms();
        let target = next_local_8am_epoch_ms(now);
        assert!(target > now, "target={target} must be > now={now}");
        // At most 25 hours ahead (accounts for a 25-hour fall-back DST day)
        assert!(
            target - now <= 25 * 3_600_000,
            "delta {}ms exceeds 25h",
            target - now
        );
        assert_eq!(target % 60_000, 0, "8:00:00 is on a minute boundary");

        // A time beyond 32-bit `time_t` (year 2100) must still give a target within a day, not a wrapped one.
        let far = days_from_civil(2100, 6, 15) * 86_400_000 + 3_600_000;
        let far_target = next_local_8am_epoch_ms(far);
        assert!(far_target > far && far_target - far <= 25 * 3_600_000, "far={far} target={far_target}");

        // UTC fallback check at 07:00 UTC -> 08:00 UTC same day (+1h)
        let utc_7am = days_from_civil(2026, 6, 15) * 86_400_000 + 7 * 3_600_000;
        assert_eq!(
            next_utc_8am_epoch_ms(utc_7am),
            utc_7am + 3_600_000
        );
        // At 08:00 UTC -> 08:00 UTC next day (+24h)
        let utc_8am = utc_7am + 3_600_000;
        assert_eq!(
            next_utc_8am_epoch_ms(utc_8am),
            utc_8am + 24 * 3_600_000
        );
    }
}
