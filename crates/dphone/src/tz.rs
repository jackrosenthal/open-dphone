//! Local time without a zone database on the phone: a compiled-in table of
//! IANA zone names and their POSIX TZ rules (the footer of each zone's TZif
//! file; regenerate with `cargo xtask tz-table`), and an evaluator for
//! those rules.  Only the current rule of each zone is known, which is what
//! a phone showing the present time needs.

/// `Zone/Name<TAB>POSIX-TZ` lines.
const TABLE: &str = include_str!("tzdata.tsv");

/// The POSIX TZ rule for an IANA zone name.
pub fn rule_for(zone: &str) -> Option<&'static str> {
    let zone = zone.trim();
    TABLE.lines().find_map(|line| {
        let (name, rule) = line.split_once('\t')?;
        name.eq_ignore_ascii_case(zone).then_some(rule)
    })
}

/// Seconds east of UTC at `unix` for a setting value: an IANA name, or a
/// POSIX TZ string given directly.  Unknown values are UTC.
pub fn utc_offset(setting: &str, unix: i64) -> i32 {
    let rule = rule_for(setting).unwrap_or(setting.trim());
    Rule::parse(rule).map(|r| r.offset_at(unix)).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Day {
    /// `Mm.w.d`: month 1..=12, week 1..=5 (5 = last), weekday 0 = Sunday.
    MonthWeek(u8, u8, u8),
    /// `Jn`: day 1..=365, February 29 never counted.
    Julian(u16),
    /// `n`: day 0..=365, February 29 counted.
    ZeroBased(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Change {
    day: Day,
    /// Local time of the change, in seconds (may be negative or over a day).
    time: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rule {
    /// Seconds east of UTC.
    std: i32,
    dst: Option<(i32, Change, Change)>,
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn year_of(unix: i64) -> i64 {
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    if mp >= 10 { y + 1 } else { y }
}

impl Change {
    /// The change as seconds since the epoch in local wall time of `year`.
    fn local_secs(&self, year: i64) -> i64 {
        let jan1 = days_from_civil(year, 1, 1);
        let day = match self.day {
            Day::Julian(n) => {
                let n = n as i64;
                jan1 + n - 1 + if is_leap(year) && n >= 60 { 1 } else { 0 }
            }
            Day::ZeroBased(n) => jan1 + n as i64,
            Day::MonthWeek(m, w, d) => {
                let first = days_from_civil(year, m as i64, 1);
                // 1970-01-01 was a Thursday (4).
                let weekday = (first + 4).rem_euclid(7);
                let mut day = first + (d as i64 - weekday).rem_euclid(7) + (w as i64 - 1) * 7;
                let next_month = if m == 12 { days_from_civil(year + 1, 1, 1) } else { days_from_civil(year, m as i64 + 1, 1) };
                while day >= next_month {
                    day -= 7;
                }
                day
            }
        };
        day * 86_400 + self.time as i64
    }
}

struct Parser<'a>(&'a str);

impl<'a> Parser<'a> {
    fn name(&mut self) -> Option<()> {
        if let Some(rest) = self.0.strip_prefix('<') {
            let end = rest.find('>')?;
            self.0 = &rest[end + 1..];
            return Some(());
        }
        let n = self.0.bytes().take_while(u8::is_ascii_alphabetic).count();
        if n < 3 {
            return None;
        }
        self.0 = &self.0[n..];
        Some(())
    }

    fn number(&mut self) -> Option<i32> {
        let n = self.0.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        let v = self.0[..n].parse().ok()?;
        self.0 = &self.0[n..];
        Some(v)
    }

    /// `[+-]hh[:mm[:ss]]` as seconds.
    fn time(&mut self) -> Option<i32> {
        let negative = match self.0.as_bytes().first()? {
            b'-' => {
                self.0 = &self.0[1..];
                true
            }
            b'+' => {
                self.0 = &self.0[1..];
                false
            }
            _ => false,
        };
        let mut secs = self.number()? * 3600;
        if let Some(rest) = self.0.strip_prefix(':') {
            self.0 = rest;
            secs += self.number()? * 60;
            if let Some(rest) = self.0.strip_prefix(':') {
                self.0 = rest;
                secs += self.number()?;
            }
        }
        Some(if negative { -secs } else { secs })
    }

    fn change(&mut self) -> Option<Change> {
        self.0 = self.0.strip_prefix(',')?;
        let day = if let Some(rest) = self.0.strip_prefix('M') {
            self.0 = rest;
            let m = self.number()?;
            self.0 = self.0.strip_prefix('.')?;
            let w = self.number()?;
            self.0 = self.0.strip_prefix('.')?;
            let d = self.number()?;
            if !(1..=12).contains(&m) || !(1..=5).contains(&w) || !(0..=6).contains(&d) {
                return None;
            }
            Day::MonthWeek(m as u8, w as u8, d as u8)
        } else if let Some(rest) = self.0.strip_prefix('J') {
            self.0 = rest;
            Day::Julian(self.number()? as u16)
        } else {
            Day::ZeroBased(self.number()? as u16)
        };
        let time = if let Some(rest) = self.0.strip_prefix('/') {
            self.0 = rest;
            self.time()?
        } else {
            7200
        };
        Some(Change { day, time })
    }
}

impl Rule {
    fn parse(text: &str) -> Option<Rule> {
        let mut p = Parser(text.trim());
        p.name()?;
        // POSIX offsets are west-positive.
        let std = -p.time()?;
        if p.0.is_empty() {
            return Some(Rule { std, dst: None });
        }
        p.name()?;
        let dst = if p.0.starts_with(',') || p.0.is_empty() { std + 3600 } else { -p.time()? };
        if p.0.is_empty() {
            // No rule given: the United States rule, as POSIX libraries assume.
            let start = Change { day: Day::MonthWeek(3, 2, 0), time: 7200 };
            let end = Change { day: Day::MonthWeek(11, 1, 0), time: 7200 };
            return Some(Rule { std, dst: Some((dst, start, end)) });
        }
        let start = p.change()?;
        let end = p.change()?;
        Some(Rule { std, dst: Some((dst, start, end)) })
    }

    fn offset_at(&self, unix: i64) -> i32 {
        let Some((dst, start, end)) = self.dst else { return self.std };
        let year = year_of(unix + self.std as i64);
        // Changes are given in the local time in force before them.
        let start_utc = start.local_secs(year) - self.std as i64;
        let end_utc = end.local_secs(year) - dst as i64;
        let in_dst = if start_utc <= end_utc {
            unix >= start_utc && unix < end_utc
        } else {
            // Southern hemisphere: summer time spans the new year.
            unix >= start_utc || unix < end_utc
        };
        if in_dst { dst } else { self.std }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i64, m: i64, d: i64, h: i64) -> i64 {
        days_from_civil(y, m, d) * 86_400 + h * 3600
    }

    #[test]
    fn fixed_and_named() {
        assert_eq!(utc_offset("UTC0", 0), 0);
        assert_eq!(utc_offset("<+0530>-5:30", 0), 19_800);
        assert_eq!(utc_offset("nonsense", 0), 0);
    }

    #[test]
    fn united_states() {
        let tz = "PST8PDT,M3.2.0,M11.1.0";
        assert_eq!(utc_offset(tz, at(2026, 1, 15, 12)), -8 * 3600);
        assert_eq!(utc_offset(tz, at(2026, 7, 15, 12)), -7 * 3600);
        // 2026-03-08 02:00 PST = 10:00 UTC.
        assert_eq!(utc_offset(tz, at(2026, 3, 8, 9)), -8 * 3600);
        assert_eq!(utc_offset(tz, at(2026, 3, 8, 10)), -7 * 3600);
        // 2026-11-01 02:00 PDT = 09:00 UTC.
        assert_eq!(utc_offset(tz, at(2026, 11, 1, 8)), -7 * 3600);
        assert_eq!(utc_offset(tz, at(2026, 11, 1, 9)), -8 * 3600);
    }

    #[test]
    fn europe_and_south() {
        let berlin = "CET-1CEST,M3.5.0,M10.5.0/3";
        assert_eq!(utc_offset(berlin, at(2026, 3, 29, 0)), 3600);
        assert_eq!(utc_offset(berlin, at(2026, 3, 29, 1)), 7200);
        assert_eq!(utc_offset(berlin, at(2026, 10, 25, 0)), 7200);
        assert_eq!(utc_offset(berlin, at(2026, 10, 25, 1)), 3600);
        let sydney = "AEST-10AEDT,M10.1.0,M4.1.0/3";
        assert_eq!(utc_offset(sydney, at(2026, 1, 15, 0)), 11 * 3600);
        assert_eq!(utc_offset(sydney, at(2026, 7, 15, 0)), 10 * 3600);
    }

    #[test]
    fn table() {
        assert_eq!(rule_for("America/Los_Angeles"), Some("PST8PDT,M3.2.0,M11.1.0"));
        assert_eq!(utc_offset("America/Denver", at(2026, 11, 13, 12)), -7 * 3600);
        assert!(rule_for("No/Such_Zone").is_none());
        // Every rule in the table parses.
        for line in TABLE.lines() {
            let (name, rule) = line.split_once('\t').unwrap();
            assert!(Rule::parse(rule).is_some(), "{name}: {rule}");
        }
    }
}
