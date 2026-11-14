//! Trial-boot bookkeeping (section 12).
//!
//! boot2 never falls back by itself.  After an update, the updater records a
//! pending trial here.  Every boot of the trial counts; when the main role
//! reports healthy the trial is confirmed and the record removed; when the
//! count runs out, or the main role keeps failing, the init role asks the
//! updater's hook to flip `ctrl` back to the previous bank and reboots.
//!
//! The record is a small text file, `/nvdata/open/trial`:
//!
//! ```text
//! pending=1
//! previous_bank=0
//! boots=1
//! max_boots=3
//! ```

use std::io;
use std::path::{Path, PathBuf};

/// Boots a trial gets before rollback, unless the record says otherwise.
pub const DEFAULT_MAX_BOOTS: u32 = 3;

/// How long a trial boot may take to report healthy before the init role
/// restarts the phone (the boot stays counted against the trial).
pub const TRIAL_HEALTHY_SECS: u64 = 300;

/// Main-role failures within one trial boot that trigger rollback at once.
pub const MAX_TRIAL_CRASHES: u32 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialRecord {
    /// The bank (0 or 1) that was active before the update; rollback target.
    pub previous_bank: u8,
    /// Boots started so far in this trial.
    pub boots: u32,
    pub max_boots: u32,
}

/// What the init role calls to go back.  Implemented by the updater crate
/// (`ctrl` record rewrite); it must only return `Ok` once the previous bank
/// is committed as active, replicated, and re-read.
pub type RollbackHook = fn(&TrialRecord) -> io::Result<()>;

pub fn record_path(open_dir: &Path) -> PathBuf {
    open_dir.join("trial")
}

impl TrialRecord {
    pub fn new(previous_bank: u8) -> Self {
        TrialRecord { previous_bank, boots: 0, max_boots: DEFAULT_MAX_BOOTS }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut pending = false;
        let mut record = TrialRecord::new(0);
        let mut have_bank = false;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "pending" => pending = value == "1",
                "previous_bank" => {
                    record.previous_bank = value.parse().ok().filter(|b| *b <= 1)?;
                    have_bank = true;
                }
                "boots" => record.boots = value.parse().ok()?,
                "max_boots" => record.max_boots = value.parse().ok()?,
                _ => {}
            }
        }
        (pending && have_bank).then_some(record)
    }

    pub fn to_text(&self) -> String {
        format!(
            "pending=1\nprevious_bank={}\nboots={}\nmax_boots={}\n",
            self.previous_bank, self.boots, self.max_boots
        )
    }

    pub fn exhausted(&self) -> bool {
        self.boots > self.max_boots
    }
}

/// The pending trial, if any.
pub fn load(open_dir: &Path) -> Option<TrialRecord> {
    TrialRecord::parse(&std::fs::read_to_string(record_path(open_dir)).ok()?)
}

pub fn store(open_dir: &Path, record: &TrialRecord) -> io::Result<()> {
    std::fs::create_dir_all(open_dir)?;
    crate::paths::write_atomic(&record_path(open_dir), record.to_text().as_bytes(), 0o644)
}

/// Remove the record: the trial is confirmed, or abandoned after rollback.
pub fn clear(open_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(record_path(open_dir)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// For the updater: start a trial after committing a new bank.
pub fn begin(open_dir: &Path, previous_bank: u8) -> io::Result<()> {
    store(open_dir, &TrialRecord::new(previous_bank))
}

/// Count this boot against a pending trial.  Returns the updated record.
pub fn count_boot(open_dir: &Path) -> Option<TrialRecord> {
    let mut record = load(open_dir)?;
    record.boots += 1;
    if let Err(e) = store(open_dir, &record) {
        log::warn!("trial record: {e}");
    }
    Some(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), None);
        assert_eq!(count_boot(dir.path()), None);
        begin(dir.path(), 1).unwrap();
        for boot in 1..=DEFAULT_MAX_BOOTS {
            let r = count_boot(dir.path()).unwrap();
            assert_eq!((r.previous_bank, r.boots), (1, boot));
            assert!(!r.exhausted());
        }
        assert!(count_boot(dir.path()).unwrap().exhausted());
        clear(dir.path()).unwrap();
        clear(dir.path()).unwrap();
        assert_eq!(load(dir.path()), None);
    }

    #[test]
    fn parse_rejects_bad_records() {
        assert_eq!(TrialRecord::parse(""), None);
        assert_eq!(TrialRecord::parse("pending=0\nprevious_bank=1\n"), None);
        assert_eq!(TrialRecord::parse("pending=1\n"), None);
        assert_eq!(TrialRecord::parse("pending=1\nprevious_bank=7\n"), None);
        let r = TrialRecord::parse("pending=1\nprevious_bank=0\nboots=2\nmax_boots=4\n").unwrap();
        assert_eq!(r, TrialRecord { previous_bank: 0, boots: 2, max_boots: 4 });
        assert_eq!(TrialRecord::parse(&r.to_text()), Some(r));
    }
}
