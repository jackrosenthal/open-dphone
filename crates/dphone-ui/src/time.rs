//! Calendar arithmetic for the clock, integer-only and without a time-zone
//! database: the integrator supplies the UTC offset.

use crate::strings as tr;

/// A broken-down local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i32,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
    /// 0 = Sunday
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Civil {
    /// Converts Unix seconds plus a UTC offset in seconds.
    pub fn from_unix(unix: u64, utc_offset_secs: i32) -> Civil {
        let t = unix as i64 + utc_offset_secs as i64;
        let days = t.div_euclid(86_400);
        let secs = t.rem_euclid(86_400);
        // Howard Hinnant's civil-from-days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = (yoe + era * 400 + if month <= 2 { 1 } else { 0 }) as i32;
        Civil {
            year,
            month,
            day,
            weekday: (days + 4).rem_euclid(7) as u8,
            hour: (secs / 3600) as u8,
            minute: (secs / 60 % 60) as u8,
            second: (secs % 60) as u8,
        }
    }

    /// `9:41` (12-hour) or `09:41` (24-hour).
    pub fn time(&self, h24: bool) -> String {
        if h24 {
            format!("{:02}:{:02}", self.hour, self.minute)
        } else {
            let h = match self.hour % 12 {
                0 => 12,
                h => h,
            };
            format!("{}:{:02}", h, self.minute)
        }
    }

    /// `AM` / `PM`, or empty in 24-hour mode.
    pub fn meridiem(&self, h24: bool) -> &'static str {
        match (h24, self.hour < 12) {
            (true, _) => "",
            (false, true) => "AM",
            (false, false) => "PM",
        }
    }

    /// `Mon, Nov 9`
    pub fn date(&self) -> String {
        format!(
            "{}, {} {}",
            tr::WEEKDAYS[self.weekday as usize % 7],
            tr::MONTHS[(self.month as usize + 11) % 12],
            self.day
        )
    }

    /// `Nov 9 9:41 AM` for call-log rows.
    pub fn short(&self, h24: bool) -> String {
        let m = self.meridiem(h24);
        format!(
            "{} {} {}{}{}",
            tr::MONTHS[(self.month as usize + 11) % 12],
            self.day,
            self.time(h24),
            if m.is_empty() { "" } else { " " },
            m
        )
    }
}

/// `m:ss` or `h:mm:ss`.
pub fn duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil() {
        // 2026-11-13 21:58:07 UTC was a Friday.
        let c = Civil::from_unix(1_794_607_087, 0);
        assert_eq!((c.year, c.month, c.day, c.weekday), (2026, 11, 13, 5));
        assert_eq!((c.hour, c.minute, c.second), (21, 58, 7));
        assert_eq!(c.time(false), "9:58");
        assert_eq!(c.meridiem(false), "PM");
        assert_eq!(c.date(), "Fri, Nov 13");
        // Offsets cross the date line.
        let c = Civil::from_unix(1_794_607_087, 3 * 3600);
        assert_eq!((c.day, c.weekday, c.hour), (14, 6, 0));
        assert_eq!(c.time(false), "12:58");
        let c = Civil::from_unix(0, 0);
        assert_eq!((c.year, c.month, c.day, c.weekday), (1970, 1, 1, 4));
        // Leap day.
        let c = Civil::from_unix(1_709_208_000, 0);
        assert_eq!((c.year, c.month, c.day), (2024, 2, 29));
        assert_eq!(duration(59), "0:59");
        assert_eq!(duration(3725), "1:02:05");
    }
}
