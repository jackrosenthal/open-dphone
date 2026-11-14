//! Trial boot (plan section 12): the nvdata side.
//!
//! boot2 never falls back on its own, so after an update a pending-trial
//! record is left in nvdata.  The init role (`dphone`, `init/trial.rs`)
//! counts boots against it, clears it when the main role reports healthy,
//! and calls back into [`rollback`] when the trial runs out.
//!
//! The record is the init role's text file `/nvdata/open/trial`; this
//! module writes the same keys (`pending`, `previous_bank`, `boots`,
//! `max_boots`) plus `trial_bank`, which the init role's parser ignores and
//! [`rollback`] uses to refuse a stale record.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use dphone_hal::Mtd;
use dphone_state::{Model, Store, TrialBoot, Update};

use crate::bank::{self, Bank};
use crate::error::{Result, UpdateError};
use crate::install::{Installed, select_bank};

/// Boots a trial gets (same default as the init role).
pub const DEFAULT_MAX_BOOTS: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialRecord {
    /// Rollback target: the bank that was running before the update.
    pub previous_bank: Bank,
    /// The bank on trial, when recorded.
    pub trial_bank: Option<Bank>,
    pub boots: u32,
    pub max_boots: u32,
}

pub fn record_path(open_dir: &Path) -> PathBuf {
    open_dir.join("trial")
}

impl TrialRecord {
    pub fn parse(text: &str) -> Option<Self> {
        let mut pending = false;
        let mut previous = None;
        let mut trial_bank = None;
        let (mut boots, mut max_boots) = (0, DEFAULT_MAX_BOOTS);
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "pending" => pending = value == "1",
                "previous_bank" => previous = Some(bank::from_index(value.parse().ok()?)?),
                "trial_bank" => trial_bank = value.parse().ok().and_then(bank::from_index),
                "boots" => boots = value.parse().ok()?,
                "max_boots" => max_boots = value.parse().ok()?,
                _ => {}
            }
        }
        if !pending {
            return None;
        }
        Some(TrialRecord { previous_bank: previous?, trial_bank, boots, max_boots })
    }

    pub fn to_text(&self) -> String {
        let mut s = format!(
            "pending=1\nprevious_bank={}\nboots={}\nmax_boots={}\n",
            bank::index(self.previous_bank),
            self.boots,
            self.max_boots
        );
        if let Some(t) = self.trial_bank {
            s.push_str(&format!("trial_bank={}\n", bank::index(t)));
        }
        s
    }
}

/// The pending trial, if any.
pub fn load(open_dir: &Path) -> Option<TrialRecord> {
    TrialRecord::parse(&fs::read_to_string(record_path(open_dir)).ok()?)
}

fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    fs::File::open(dir)?.sync_all()
}

/// Records a pending trial after a committed installation.  Call this
/// before rebooting into the new image.
///
/// Nothing is recorded when the image is stock firmware: stock does not
/// know the record, and one left behind would be counted against whatever
/// Open DPhone image is installed next.  Any older record is removed in
/// that case.  Returns whether a trial was started.
pub fn begin(open_dir: &Path, installed: &Installed) -> io::Result<bool> {
    if installed.is_stock {
        clear(open_dir)?;
        return Ok(false);
    }
    let record = TrialRecord {
        previous_bank: installed.previous,
        trial_bank: Some(installed.bank),
        boots: 0,
        max_boots: DEFAULT_MAX_BOOTS,
    };
    write_atomic(&record_path(open_dir), record.to_text().as_bytes())?;
    Ok(true)
}

/// Removes the record: the trial is confirmed (the main role is healthy),
/// or abandoned.
pub fn clear(open_dir: &Path) -> io::Result<()> {
    match fs::remove_file(record_path(open_dir)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Main role: the system is healthy; confirm the trial.  (The init role
/// also clears the record when the main role's heartbeat says healthy;
/// either may come first.)
pub fn mark_healthy(open_dir: &Path, store: Option<&Store>) -> io::Result<()> {
    clear(open_dir)?;
    if let Some(s) = store {
        s.update::<Update, _>(|u| {
            if matches!(u.trial, TrialBoot::Pending { .. }) {
                u.trial = TrialBoot::None;
            }
        });
    }
    Ok(())
}

/// Goes back: selects `previous` in `ctrl` so the next boot is the old
/// image.  Returns `Ok` only once the record is rewritten in the family's
/// replication scheme and reads back as selecting `previous`.
///
/// Refused when the running system is already from `previous` (a stale
/// trial record) or when `previous` is not bootable by boot2's rules; the
/// flash is then unchanged.
pub fn rollback(mtd: &mut dyn Mtd, model: Model, cmdline: &str, previous: Bank) -> Result<()> {
    let parts = mtd.partitions()?;
    if bank::running_bank(cmdline, &parts) == Some(previous) {
        return Err(UpdateError::Refused(format!(
            "already running from bank {previous:?}; the trial record is stale"
        )));
    }
    select_bank(mtd, model, previous)
}

/// Seeds the `Update` topic at start-up: running bank and trial status.
pub fn seed_state(store: &Store, open_dir: &Path, cmdline: &str, mtd: &dyn Mtd) {
    let running = mtd.partitions().ok().and_then(|p| bank::running_bank(cmdline, &p));
    let trial = match load(open_dir) {
        Some(r) if r.trial_bank.is_none() || r.trial_bank == running => TrialBoot::Pending {
            boots_left: r.max_boots.saturating_sub(r.boots).min(u8::MAX as u32) as u8,
        },
        _ => TrialBoot::None,
    };
    store.update::<Update, _>(|u| {
        u.running_bank = running;
        u.trial = trial;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_matches_the_init_roles_format() {
        let r = TrialRecord { previous_bank: Bank::A, trial_bank: Some(Bank::B), boots: 0, max_boots: 3 };
        // The four lines the init role's parser needs, in its own layout.
        assert!(r.to_text().starts_with("pending=1\nprevious_bank=0\nboots=0\nmax_boots=3\n"));
        assert_eq!(TrialRecord::parse(&r.to_text()), Some(r));
        // A record the init role rewrote (it drops keys it does not know).
        let r = TrialRecord::parse("pending=1\nprevious_bank=1\nboots=2\nmax_boots=3\n").unwrap();
        assert_eq!((r.previous_bank, r.trial_bank, r.boots), (Bank::B, None, 2));
        assert_eq!(TrialRecord::parse("pending=0\nprevious_bank=1\n"), None);
        assert_eq!(TrialRecord::parse("pending=1\n"), None);
        assert_eq!(TrialRecord::parse("pending=1\nprevious_bank=2\n"), None);
    }
}
