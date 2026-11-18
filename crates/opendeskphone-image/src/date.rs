//! The build-date text of the OTA header, in the stock style
//! (`Tue Oct 27 13:05:08 2026`, UTC).

pub fn ctime_utc(epoch: u64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = epoch / 86400;
    let secs = epoch % 86400;
    // Civil date from a day count (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{} {} {:2} {:02}:{:02}:{:02} {}",
        DAYS[(days % 7) as usize],
        MONTHS[(month - 1) as usize],
        day,
        secs / 3600,
        secs % 3600 / 60,
        secs % 60,
        year
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_dates() {
        assert_eq!(super::ctime_utc(0), "Thu Jan  1 00:00:00 1970");
        assert_eq!(super::ctime_utc(1_792_415_108), "Mon Oct 19 13:05:08 2026");
        assert_eq!(super::ctime_utc(951_782_400), "Tue Feb 29 00:00:00 2000");
    }
}
