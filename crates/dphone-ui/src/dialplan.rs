//! The dial plan (stock "digit map"): decides when the digits entered in an
//! off-hook dialer are dialed without the user pressing Dial.
//!
//! This is stock's compiler and matcher (the `phone` program, 2_9_28:
//! `0x30280` compiles, `0x2fdc4` matches one alternative, `0x303dc`
//! decides), reproduced with its results, including the ones that look
//! odd:
//!
//! - Alternatives are separated by `|`.  An alternative is a sequence of
//!   `0`-`9`, `#`, `*`, and `+` (that character), `x` or `X` (any digit),
//!   `[...]` (one of the listed digits; `a-b` is a range), an element
//!   followed by `.` (that element any number of times, including none),
//!   and at its end optionally `T` with a number of seconds (4 when no
//!   number follows): the alternative is complete only once that long has
//!   passed since the last key.
//! - A plan that does not compile is no plan at all.
//! - With digits entered: when no alternative matches or can still match,
//!   the digits are dialed at once.  While some alternative can still
//!   match and none matches completely, nothing is dialed.  A complete
//!   match is dialed at once, unless another alternative can still match:
//!   then 2 seconds after the last key.
//! - An alternative that contains `.` matches completely only when the
//!   repeated element took no digits; otherwise, once its `T` has passed
//!   it no longer matches at all, and the digits are dialed by the
//!   no-match rule.  The result for the user is the same.

/// Stock's limit on the compiled plan, in its 32-bit cells.
const MAX_CELLS: usize = 0x1400;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    /// `0`-`9`, `#`, `*`, or `+`.
    Lit(u8),
    /// `x`: any digit.
    Digit,
    /// `[...]`.
    Set(Vec<u8>),
    /// `.`, stored before the element it repeats (as stock stores it).
    Star,
    /// `T`: seconds since the last key.
    Timer(u32),
    /// `|`.
    Alt,
}

/// A compiled dial plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialPlan {
    toks: Vec<Tok>,
}

impl DialPlan {
    /// Compiles `plan`; the error is stock's message.
    pub fn compile(plan: &str) -> Result<DialPlan, String> {
        let s = plan.as_bytes();
        if s.is_empty() {
            return Err("the dial plan is empty".into());
        }
        let unexpected = |at: usize| {
            let ch = s.get(at).map_or('\0', |c| *c as char);
            format!("unexpected character at position {}: '{}'", at + 1, ch.escape_default())
        };
        let mut toks: Vec<Tok> = Vec::new();
        let mut cells = 0usize;
        let mut i = 0usize;
        loop {
            // One alternative.
            let mut first = true;
            // Where the last element starts: `.` goes in before it.
            let mut last = toks.len();
            loop {
                let c = s.get(i).copied().unwrap_or(0);
                match c {
                    b'0'..=b'9' | b'#' | b'*' | b'+' => {
                        last = toks.len();
                        toks.push(Tok::Lit(c));
                        cells += 1;
                        i += 1;
                    }
                    b'x' | b'X' => {
                        last = toks.len();
                        toks.push(Tok::Digit);
                        cells += 1;
                        i += 1;
                    }
                    b'.' => {
                        // Stock accepts a `.` with no element before it in
                        // every alternative but the first and then reads
                        // past the alternative; it is an error here.
                        if first {
                            return Err(unexpected(i));
                        }
                        // A second `.` on the same element changes nothing.
                        if last == 0 || toks[last - 1] != Tok::Star {
                            toks.insert(last, Tok::Star);
                            cells += 1;
                            last += 1;
                        }
                        i += 1;
                    }
                    b'[' => {
                        last = toks.len();
                        i += 1;
                        if !s.get(i).is_some_and(u8::is_ascii_digit) {
                            return Err(unexpected(i));
                        }
                        let mut set = Vec::new();
                        while let Some(&lo) = s.get(i).filter(|c| c.is_ascii_digit()) {
                            match (s.get(i + 1), s.get(i + 2)) {
                                (Some(b'-'), Some(&hi)) if hi.is_ascii_digit() => {
                                    if hi < lo {
                                        return Err(format!("invalid range starting at position {i}"));
                                    }
                                    set.extend(lo..=hi);
                                    i += 3;
                                }
                                _ => {
                                    set.push(lo);
                                    i += 1;
                                }
                            }
                        }
                        if s.get(i) != Some(&b']') {
                            return Err(unexpected(i));
                        }
                        cells += 2 + set.len();
                        toks.push(Tok::Set(set));
                        i += 1;
                    }
                    _ if first => return Err(unexpected(i)),
                    b'T' => {
                        i += 1;
                        let mut secs = 4u32;
                        if s.get(i).is_some_and(u8::is_ascii_digit) {
                            secs = 0;
                            while let Some(d) = s.get(i).filter(|c| c.is_ascii_digit()) {
                                secs = secs.saturating_mul(10).saturating_add(u32::from(d - b'0'));
                                i += 1;
                            }
                        }
                        toks.push(Tok::Timer(secs));
                        cells += 2;
                        break;
                    }
                    _ => break,
                }
                first = false;
            }
            match s.get(i) {
                Some(b'|') => {
                    toks.push(Tok::Alt);
                    cells += 1;
                    i += 1;
                }
                None if cells < MAX_CELLS => return Ok(DialPlan { toks }),
                None => return Err("the compiled dial plan exceeds the allocated memory".into()),
                Some(_) => return Err(unexpected(i)),
            }
        }
    }

    fn at_end(&self, i: usize) -> bool {
        matches!(self.toks.get(i), None | Some(Tok::Alt))
    }

    /// One element against the input at `pos`: whether it matches, and
    /// whether it took a character.  `timed` says whether this element was
    /// a timer that has passed.
    fn element(&self, s: &[u8], pos: usize, i: usize, elapsed: u32, timed: &mut bool) -> Option<bool> {
        *timed = false;
        let c = s.get(pos).copied();
        match self.toks.get(i)? {
            Tok::Lit(l) => (c == Some(*l)).then_some(true),
            Tok::Digit => c.is_some_and(|c| c.is_ascii_digit()).then_some(true),
            Tok::Set(set) => c.is_some_and(|c| set.contains(&c)).then_some(true),
            Tok::Timer(secs) if *secs <= elapsed => {
                *timed = true;
                Some(false)
            }
            Tok::Timer(_) | Tok::Star | Tok::Alt => None,
        }
    }

    /// Matches the alternative from token `i` against the input from
    /// `pos`.  Returns where the input stands (`None`: the input has a
    /// character the alternative does not accept) and the token reached.
    fn run(&self, s: &[u8], mut pos: usize, mut i: usize, elapsed: u32, timed: &mut bool) -> (Option<usize>, usize) {
        loop {
            if self.at_end(i) {
                return (Some(pos), i);
            }
            if self.toks[i] != Tok::Star {
                match self.element(s, pos, i, elapsed, timed) {
                    None => return ((pos >= s.len()).then_some(pos), i),
                    Some(took) => pos += usize::from(took),
                }
                i += 1;
                continue;
            }
            // As many of the repeated element as there are, then fewer,
            // until the rest of the alternative matches to its end.
            let mut hi = pos;
            while self.element(s, hi, i + 1, elapsed, timed) == Some(true) {
                hi += 1;
            }
            let rest = i + 2;
            let mut reached = rest;
            for k in (pos..=hi).rev() {
                let (_, r) = self.run(s, k, rest, elapsed, timed);
                if self.at_end(r) {
                    reached = r;
                    break;
                }
            }
            // Stock reports the input position at the `.`, not after the
            // digits the repeated element took.
            return (Some(pos), reached);
        }
    }

    /// Whether `digits` are to be dialed now, `elapsed_secs` whole seconds
    /// after the last key.
    pub fn dial_now(&self, digits: &str, elapsed_secs: u32) -> bool {
        let s = digits.as_bytes();
        if s.is_empty() {
            return false;
        }
        let mut complete = false;
        let mut partial = false;
        let mut start = 0usize;
        while start < self.toks.len() {
            let mut timed = false;
            let (pos, reached) = self.run(s, 0, start, elapsed_secs, &mut timed);
            if let Some(pos) = pos {
                if pos >= s.len() && self.at_end(reached) {
                    if timed {
                        return true;
                    }
                    complete = true;
                } else if !self.at_end(reached) && reached > start {
                    partial = true;
                }
            }
            start = reached;
            while !self.at_end(start) {
                start += 1;
            }
            start += 1;
        }
        match (complete, partial) {
            (true, false) => true,
            (true, true) => elapsed_secs > 1,
            (false, true) => false,
            (false, false) => true,
        }
    }

    /// The first whole second, at or after `elapsed_secs` since the last
    /// key, at which `digits` are dialed with no further key; `None` when
    /// they wait for another key.
    pub fn dial_after(&self, digits: &str, elapsed_secs: u32) -> Option<u32> {
        let mut at: Vec<u32> = vec![elapsed_secs, 2];
        at.extend(self.toks.iter().filter_map(|t| match t {
            Tok::Timer(secs) => Some(*secs),
            _ => None,
        }));
        at.retain(|s| *s >= elapsed_secs);
        at.sort_unstable();
        at.into_iter().find(|s| self.dial_now(digits, *s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plan the PBX provisions for the owner's lines.
    const PBX: &str = "0xx|[2-9]xxxxxxxxx|1[2-9]xxxxxxxxxx";
    /// Stock's `default_dial_plan`.
    const STOCK: &str = "[0-8]xx|911|9411|9611|9011xxx.T3|91xxxxxxxxxx|9[2-9]xxxxxx|*xxx.T3|+xxxxxxxxxxxxxxx";

    fn plan(s: &str) -> DialPlan {
        DialPlan::compile(s).unwrap()
    }

    /// The length of the shortest prefix of `digits` that is dialed at
    /// once (no time passing).
    fn dials_at(p: &DialPlan, digits: &str) -> Option<usize> {
        (1..=digits.len()).find(|n| p.dial_now(&digits[..*n], 0))
    }

    #[test]
    fn the_provisioned_plan_wants_twelve_digits_after_a_one() {
        // `1[2-9]xxxxxxxxxx` is 1, [2-9], and ten more digits: 18004377950
        // (eleven digits) can still match and is not complete, so the plan
        // never dials it, however long the user waits.
        let p = plan(PBX);
        let number = "18004377950";
        for n in 1..=number.len() {
            assert!(!p.dial_now(&number[..n], 0), "dialed at {}", &number[..n]);
            assert_eq!(p.dial_after(&number[..n], 0), None);
            assert!(!p.dial_now(&number[..n], 3600));
        }
        assert!(p.dial_now("180043779501", 0));
    }

    #[test]
    fn eleven_digits_dial_at_the_last_digit_with_nine_after_the_area_code_digit() {
        let p = plan("0xx|[2-9]xxxxxxxxx|1[2-9]xxxxxxxxx");
        assert_eq!(dials_at(&p, "18004377950"), Some(11));
        assert_eq!(p.dial_after("18004377950", 0), Some(0));
        assert_eq!(p.dial_after("1800437795", 0), None);
    }

    #[test]
    fn the_provisioned_plan_dials_extensions_and_local_numbers() {
        let p = plan(PBX);
        assert_eq!(dials_at(&p, "021"), Some(3));
        assert_eq!(dials_at(&p, "2565551234"), Some(10));
    }

    #[test]
    fn digits_no_alternative_can_match_are_dialed_at_once() {
        let p = plan(PBX);
        // `1` can still become `1[2-9]...`; `11` cannot become anything.
        assert!(!p.dial_now("1", 0));
        assert!(p.dial_now("11", 0));
        assert!(p.dial_now("*", 0));
        assert!(p.dial_now("#", 0));
    }

    #[test]
    fn nothing_entered_is_never_dialed() {
        assert!(!plan(PBX).dial_now("", 100));
        assert_eq!(plan(PBX).dial_after("", 0), None);
    }

    #[test]
    fn stock_default_plan() {
        let p = plan(STOCK);
        assert_eq!(dials_at(&p, "100"), Some(3));
        // `911` and `9411` are complete, but `91xxxxxxxxxx` and
        // `9[2-9]xxxxxx` could still match: 2 seconds of quiet.
        for n in ["911", "9411", "9611"] {
            assert!(!p.dial_now(n, 0), "{n}");
            assert_eq!(p.dial_after(n, 0), Some(2), "{n}");
        }
        assert_eq!(dials_at(&p, "918004377950"), Some(12));
        assert_eq!(dials_at(&p, "95551234"), Some(8));
        assert_eq!(dials_at(&p, "+123456789012345"), Some(16));
        // `9[2-9]xxxxxx` could still match `92`; `90` could be `9011...`.
        assert!(!p.dial_now("92", 0));
        assert!(!p.dial_now("90", 0));
        assert!(p.dial_now("99999999", 0));
    }

    #[test]
    fn a_timer_dials_after_its_seconds() {
        let p = plan(STOCK);
        // `*xxx.T3`: two digits at least, then 3 seconds of quiet.
        assert!(!p.dial_now("*7", 10));
        assert_eq!(p.dial_after("*7", 0), None);
        for code in ["*72", "*721", "*7215551234"] {
            assert!(!p.dial_now(code, 0), "{code}");
            assert!(!p.dial_now(code, 2), "{code}");
            assert!(p.dial_now(code, 3), "{code}");
            assert_eq!(p.dial_after(code, 0), Some(3), "{code}");
        }
        assert_eq!(p.dial_after("901144", 0), Some(3));
        assert_eq!(p.dial_after("9011442071234567", 1), Some(3));
        // `T` alone is 4 seconds.
        let p = plan("xxT|#");
        assert_eq!(p.dial_after("12", 0), Some(4));
        assert!(p.dial_now("#", 0));
    }

    #[test]
    fn a_complete_match_waits_two_seconds_while_a_longer_one_is_possible() {
        let p = plan("1xx|1xxx");
        assert!(!p.dial_now("123", 0));
        assert!(!p.dial_now("123", 1));
        assert!(p.dial_now("123", 2));
        assert_eq!(p.dial_after("123", 0), Some(2));
        assert!(p.dial_now("1234", 0));
    }

    #[test]
    fn sets_list_digits_and_ranges() {
        let p = plan("[13-58]x");
        for d in ["10", "30", "40", "50", "80"] {
            assert_eq!(dials_at(&p, d), Some(2), "{d}");
        }
        // Not in the set: no alternative can match, so dialed at once.
        for d in ["2", "6", "9"] {
            assert!(p.dial_now(d, 0), "{d}");
        }
    }

    #[test]
    fn a_repeat_may_take_no_digits_and_a_double_dot_is_one() {
        assert_eq!(plan("9x.").toks, plan("9x..").toks);
        let p = plan("9x.#");
        assert!(p.dial_now("9#", 0));
        assert!(!p.dial_now("9", 0));
        assert!(!p.dial_now("95", 0));
    }

    #[test]
    fn plans_that_do_not_compile() {
        for bad in ["", "|", "1xx|", "|1xx", "1xx||2xx", "1 xx", ".x", "1xx|.x", "T3", "[a]", "[]", "[9-1]", "[12", "xxT3x", "1xxy", "xx,yy"] {
            assert!(DialPlan::compile(bad).is_err(), "{bad:?} compiled");
        }
        assert!(DialPlan::compile(&format!("{}1", "1|".repeat(3000))).is_err());
        assert!(DialPlan::compile(&format!("{}1", "1|".repeat(2000))).is_ok());
        assert!(DialPlan::compile("[0-8]xx|#|*x.T|+1").is_ok());
    }
}
