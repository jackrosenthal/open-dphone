//! Wall-clock save and restore.  No model has a real-time clock, so the time
//! is kept in `/nvdata/systime` across reboots until SNTP corrects it.
//!
//! The file keeps the stock form, `Nov 12 16:14:43 2026`, which stock feeds to
//! `date -s`.  We write and read it as UTC.  Stock reads it in its own fixed
//! zone, so after a flash back its temporary clock is a few hours off until
//! NTP runs, which is harmless.

use std::io;
use std::path::Path;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Days since 1970-01-01 for a proleptic Gregorian date.
pub const fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// (year, month, day) for days since 1970-01-01.
pub const fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// The clock is never set earlier than this: 2026-11-01 00:00:00 UTC.
pub const FLOOR: i64 = days_from_civil(2026, 11, 1) * 86400;

/// Format seconds since the epoch in the stock `systime` form (UTC).
pub fn format_systime(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{} {} {:02}:{:02}:{:02} {}", MONTHS[(m - 1) as usize], d, rem / 3600, rem % 3600 / 60, rem % 60, y)
}

/// Parse the stock `systime` form; also accepts a bare count of seconds.
pub fn parse_systime(text: &str) -> Option<i64> {
    let text = text.trim();
    if let Ok(secs) = text.parse::<i64>() {
        return Some(secs);
    }
    let mut parts = text.split_ascii_whitespace();
    let month = parts.next()?;
    let month = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(month))? as u32 + 1;
    let day: u32 = parts.next()?.parse().ok()?;
    let mut hms = parts.next()?.split(':');
    let h: i64 = hms.next()?.parse().ok()?;
    let mi: i64 = hms.next()?.parse().ok()?;
    let s: i64 = hms.next().unwrap_or("0").parse().ok()?;
    let year: i64 = parts.next()?.parse().ok()?;
    if !(1..=31).contains(&day) || !(0..24).contains(&h) || !(0..60).contains(&mi) || !(0..=60).contains(&s) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86400 + h * 3600 + mi * 60 + s)
}

/// The time to start from: the saved time or the floor, whichever is later.
pub fn restore_target(saved: Option<&str>) -> i64 {
    saved.and_then(parse_systime).unwrap_or(FLOOR).max(FLOOR)
}

/// Save the current time.  A clock still at or before the floor is not worth
/// saving (nothing has corrected it), unless no file exists yet.
pub fn save(path: &Path, now: i64) -> io::Result<()> {
    let mut text = format_systime(now);
    text.push('\n');
    crate::paths::write_atomic(path, text.as_bytes(), 0o644)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        // 2026-11-12 16:14:43 UTC
        let secs = days_from_civil(2026, 11, 12) * 86400 + 16 * 3600 + 14 * 60 + 43;
        assert_eq!(format_systime(secs), "Nov 12 16:14:43 2026");
        assert_eq!(parse_systime("Nov 12 16:14:43 2026\n"), Some(secs));
        assert_eq!(parse_systime(&secs.to_string()), Some(secs));
        assert_eq!(parse_systime("garbage"), None);
    }

    #[test]
    fn known_epochs() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(civil_from_days(11017), (2000, 3, 1));
        assert_eq!(format_systime(0), "Jan 1 00:00:00 1970");
        assert_eq!(FLOOR, 1793491200);
    }

    #[test]
    fn floor_applies() {
        assert_eq!(restore_target(None), FLOOR);
        assert_eq!(restore_target(Some("Jan 1 00:00:10 1970")), FLOOR);
        assert!(restore_target(Some("Nov 12 16:14:43 2026")) > FLOOR);
    }
}
