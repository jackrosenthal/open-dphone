//! The trial-boot record: the text file `/nvdata/open/trial`.
//!
//! ```text
//! pending=1
//! previous_bank=0
//! boots=1
//! max_boots=3
//! trial_bank=1
//! ```
//!
//! The updater writes it after it has selected a newly installed bank.
//! The early-boot stage counts every boot of that bank against it and
//! selects `previous_bank` again when the count runs out (or at once when
//! the new root file system cannot be mounted or has no init to run).
//! The init role removes it when the main role reports healthy.

/// Boots a trial gets.
pub const DEFAULT_MAX_BOOTS: u32 = 3;

/// The longest text [`Trial::format`] produces.
pub const TEXT_MAX: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trial {
    /// Rollback target (0 or 1): the bank that ran before the update.
    pub previous_bank: u8,
    /// The bank on trial, when recorded.
    pub trial_bank: Option<u8>,
    /// Boots started so far.
    pub boots: u32,
    pub max_boots: u32,
}

fn number(text: &[u8]) -> Option<u32> {
    if text.is_empty() || text.len() > 9 {
        return None;
    }
    let mut n = 0u32;
    for &b in text {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (b - b'0') as u32;
    }
    Some(n)
}

fn bank(text: &[u8]) -> Option<u8> {
    number(text).filter(|n| *n <= 1).map(|n| n as u8)
}

impl Trial {
    /// The pending trial in the file's text; `None` when there is none or
    /// the text is damaged.
    pub fn parse(text: &[u8]) -> Option<Trial> {
        let mut pending = false;
        let mut previous = None;
        let mut trial_bank = None;
        let (mut boots, mut max_boots) = (0, DEFAULT_MAX_BOOTS);
        for line in text.split(|b| *b == b'\n') {
            let Some(eq) = line.iter().position(|b| *b == b'=') else { continue };
            let (key, value) = (line[..eq].trim_ascii(), line[eq + 1..].trim_ascii());
            match key {
                b"pending" => pending = value == b"1",
                b"previous_bank" => previous = Some(bank(value)?),
                b"trial_bank" => trial_bank = bank(value),
                b"boots" => boots = number(value)?,
                b"max_boots" => max_boots = number(value)?,
                _ => {}
            }
        }
        if !pending {
            return None;
        }
        Some(Trial { previous_bank: previous?, trial_bank, boots, max_boots })
    }

    /// Writes the text form into `out`; returns its length.
    pub fn format(&self, out: &mut [u8; TEXT_MAX]) -> usize {
        let mut n = 0;
        let mut put = |s: &[u8]| {
            out[n..n + s.len()].copy_from_slice(s);
            n += s.len();
        };
        let mut line = |key: &[u8], value: u32| {
            let mut digits = [0u8; 10];
            let mut i = digits.len();
            let mut v = value;
            loop {
                i -= 1;
                digits[i] = b'0' + (v % 10) as u8;
                v /= 10;
                if v == 0 {
                    break;
                }
            }
            put(key);
            put(b"=");
            put(&digits[i..]);
            put(b"\n");
        };
        line(b"pending", 1);
        line(b"previous_bank", self.previous_bank as u32);
        line(b"boots", self.boots);
        line(b"max_boots", self.max_boots);
        if let Some(t) = self.trial_bank {
            line(b"trial_bank", t as u32);
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_limits() {
        let t = Trial { previous_bank: 1, trial_bank: Some(0), boots: 2, max_boots: 3 };
        let mut buf = [0u8; TEXT_MAX];
        let n = t.format(&mut buf);
        assert_eq!(&buf[..n], b"pending=1\nprevious_bank=1\nboots=2\nmax_boots=3\ntrial_bank=0\n");
        assert_eq!(Trial::parse(&buf[..n]), Some(t));
        // The longest record fits.
        let t = Trial { previous_bank: 1, trial_bank: Some(1), boots: u32::MAX, max_boots: u32::MAX };
        let n = t.format(&mut buf);
        assert!(n <= TEXT_MAX);
        // Counts that large are not read back: the record is damaged.
        assert_eq!(Trial::parse(&buf[..n]), None);
        // A record the init role of an older image rewrote (no trial_bank).
        let t = Trial::parse(b"pending=1\nprevious_bank=0\nboots=1\nmax_boots=3\n").unwrap();
        assert_eq!((t.previous_bank, t.trial_bank, t.boots, t.max_boots), (0, None, 1, 3));
        for bad in [&b""[..], b"pending=0\nprevious_bank=1\n", b"pending=1\n", b"pending=1\nprevious_bank=2\n", b"pending=1\nprevious_bank=0\nboots=x\n"] {
            assert_eq!(Trial::parse(bad), None);
        }
    }
}
