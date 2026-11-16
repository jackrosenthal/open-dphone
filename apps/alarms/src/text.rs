//! The texts of the Alarms app that take working out: the days an alarm
//! repeats on, and a time of day as it is typed.

const SHORT_DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAYS: u8 = 0b0111110;
const WEEKEND: u8 = 0b1000001;
const EVERY_DAY: u8 = 0b1111111;

/// The days an alarm rings on (bit 0 is Sunday), in a few words.
pub fn days_label(days: u8) -> String {
    match days & EVERY_DAY {
        0 => "Once".to_string(),
        EVERY_DAY => "Every day".to_string(),
        WEEKDAYS => "Weekdays".to_string(),
        WEEKEND => "Weekends".to_string(),
        days => (0..7)
            .filter(|i| days & (1 << i) != 0)
            .map(|i| SHORT_DAYS[i])
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// A reason the phone gave, as a sentence.
pub fn sentence(why: &str) -> String {
    let mut chars = why.chars();
    let mut s: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => return String::new(),
    };
    if !s.ends_with('.') {
        s.push('.');
    }
    s
}

/// A time of day being typed: up to four digits, hour then minutes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Typed {
    digits: Vec<u8>,
    /// The afternoon half of the day, where the phone shows AM and PM.
    pub pm: bool,
    /// Nothing was typed yet: the first digit replaces the time shown.
    fresh: bool,
}

impl Typed {
    /// The digits of a time, as the phone shows it (`meridiem`: with AM
    /// and PM).
    pub fn of(hour: u8, minute: u8, meridiem: bool) -> Typed {
        let shown = match (meridiem, hour % 12) {
            (false, _) => hour,
            (true, 0) => 12,
            (true, h) => h,
        };
        Typed {
            digits: vec![shown / 10, shown % 10, minute / 10, minute % 10],
            pm: hour >= 12,
            fresh: true,
        }
    }

    pub fn push(&mut self, digit: u8) {
        if std::mem::take(&mut self.fresh) {
            self.digits.clear();
        }
        if self.digits.len() < 4 {
            self.digits.push(digit.min(9));
        }
    }

    pub fn erase(&mut self) {
        self.fresh = false;
        self.digits.pop();
    }

    /// What the screen shows: `08:3_`, with ` AM` or ` PM` after it where
    /// the phone shows those.
    pub fn text(&self, meridiem: bool) -> String {
        let d = |i: usize| self.digits.get(i).map_or('_', |d| (b'0' + d) as char);
        let mut s: String = [d(0), d(1), ':', d(2), d(3)].iter().collect();
        if meridiem {
            s.push_str(if self.pm { " PM" } else { " AM" });
        }
        s
    }

    /// The time typed, as hour (0 to 23) and minute, when it is one.  With
    /// AM and PM, hours 1 to 12 are of the half of the day chosen; 0 and
    /// 13 to 23 are taken as they are on a 24-hour clock.
    pub fn time(&self, meridiem: bool) -> Option<(u8, u8)> {
        let &[h1, h2, m1, m2] = self.digits.as_slice() else {
            return None;
        };
        let (hour, minute) = (h1 * 10 + h2, m1 * 10 + m2);
        if hour > 23 || minute > 59 {
            return None;
        }
        let hour = match (meridiem, hour) {
            (true, 12) => 12 * u8::from(self.pm),
            (true, 1..=11) => hour + 12 * u8::from(self.pm),
            _ => hour,
        };
        Some((hour, minute))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_days_in_a_few_words() {
        assert_eq!(days_label(0), "Once");
        assert_eq!(days_label(0x7f), "Every day");
        assert_eq!(days_label(0b0111110), "Weekdays");
        assert_eq!(days_label(0b1000001), "Weekends");
        assert_eq!(days_label(0b0000010), "Mon");
        assert_eq!(days_label(0b0101010), "Mon Wed Fri");
        assert_eq!(days_label(0b1111110), "Mon Tue Wed Thu Fri Sat");
    }

    #[test]
    fn reasons_become_sentences() {
        assert_eq!(sentence("the phone keeps at most 16 alarms"), "The phone keeps at most 16 alarms.");
        assert_eq!(sentence("Done."), "Done.");
        assert_eq!(sentence(""), "");
    }

    fn typed(digits: &str, pm: bool) -> Typed {
        let mut t = Typed { pm, ..Typed::default() };
        for d in digits.bytes() {
            t.push(d - b'0');
        }
        t
    }

    #[test]
    fn a_time_typed_on_a_24_hour_clock() {
        assert_eq!(typed("0800", false).time(false), Some((8, 0)));
        assert_eq!(typed("2359", false).time(false), Some((23, 59)));
        assert_eq!(typed("0000", false).time(false), Some((0, 0)));
        assert_eq!(typed("2400", false).time(false), None);
        assert_eq!(typed("0860", false).time(false), None);
        assert_eq!(typed("080", false).time(false), None);
        assert_eq!(typed("08005", false).time(false), Some((8, 0)));
    }

    #[test]
    fn a_time_typed_with_am_and_pm() {
        assert_eq!(typed("0800", false).time(true), Some((8, 0)));
        assert_eq!(typed("0800", true).time(true), Some((20, 0)));
        assert_eq!(typed("1200", false).time(true), Some((0, 0)));
        assert_eq!(typed("1230", true).time(true), Some((12, 30)));
        // Typed as on a 24-hour clock, it is what it says.
        assert_eq!(typed("1545", false).time(true), Some((15, 45)));
        assert_eq!(typed("0015", true).time(true), Some((0, 15)));
    }

    #[test]
    fn the_time_shown_is_typed_over() {
        let mut t = Typed::of(20, 5, true);
        assert_eq!(t.text(true), "08:05 PM");
        assert_eq!(t.time(true), Some((20, 5)));
        t.push(1);
        assert_eq!(t.text(true), "1_:__ PM");
        assert_eq!(t.time(true), None);
        t.push(1);
        t.push(3);
        t.erase();
        t.push(4);
        t.push(5);
        assert_eq!(t.text(true), "11:45 PM");
        assert_eq!(t.time(true), Some((23, 45)));
        // Erasing first takes the last digit of the time shown.
        let mut t = Typed::of(0, 30, true);
        assert_eq!(t.text(true), "12:30 AM");
        t.erase();
        assert_eq!(t.text(true), "12:3_ AM");
        assert_eq!(Typed::of(13, 7, false).text(false), "13:07");
        assert_eq!(Typed::of(12, 0, true).time(true), Some((12, 0)));
    }
}
