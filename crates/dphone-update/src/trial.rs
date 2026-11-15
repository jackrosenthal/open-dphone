//! Trial boot (plan section 12): the nvdata side.
//!
//! boot2 never falls back on its own, so an update leaves a pending-trial
//! record in nvdata before it selects the new bank.  The early-boot stage
//! in the new image's kernel partition (`dphone-early`) counts boots
//! against it before the root file system is needed, and selects the
//! previous bank again when the count runs out, when the root cannot be
//! mounted, or when it has no init to run.  The init role (`dphone`,
//! `init/trial.rs`) removes the record when the main role reports healthy,
//! and calls back into [`rollback`] when the main role keeps failing.
//!
//! The record is the text file `/nvdata/open/trial`; its format and the
//! rules for counting are in `dphone-ctrl`, shared with the early stage.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use dphone_hal::Mtd;
use dphone_state::{Model, Store, TrialBoot, Update};

use crate::bank::{self, Bank};
use crate::error::{Result, UpdateError};
use crate::install::select_bank;

/// Boots a trial gets (same default as the init role).
pub const DEFAULT_MAX_BOOTS: u32 = dphone_ctrl::trial::DEFAULT_MAX_BOOTS;

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
        let t = dphone_ctrl::trial::Trial::parse(text.as_bytes())?;
        Some(TrialRecord {
            previous_bank: bank::from_index(t.previous_bank as u32)?,
            trial_bank: t.trial_bank.and_then(|b| bank::from_index(b as u32)),
            boots: t.boots,
            max_boots: t.max_boots,
        })
    }

    pub fn to_text(&self) -> String {
        let t = dphone_ctrl::trial::Trial {
            previous_bank: bank::index(self.previous_bank) as u8,
            trial_bank: self.trial_bank.map(|b| bank::index(b) as u8),
            boots: self.boots,
            max_boots: self.max_boots,
        };
        let mut buf = [0u8; dphone_ctrl::trial::TEXT_MAX];
        let n = t.format(&mut buf);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

/// Whether a record is about an image that is still on trial: `ctrl`
/// selects a bank other than the one to return to.  (After the phone went
/// back, or before the commit of an update that then failed, `ctrl`
/// selects the bank to return to, and the record is a leftover.)
pub fn is_pending(record: &TrialRecord, ctrl: &eff::ctrl::CtrlRecord) -> bool {
    ctrl.boot_side() != bank::index(record.previous_bank) as u32
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

/// Records a trial of `trial_bank`, with `previous` as the bank to return
/// to.  The updater calls this just before it selects `trial_bank`.
pub fn record(open_dir: &Path, previous: Bank, trial_bank: Bank) -> io::Result<()> {
    let record =
        TrialRecord { previous_bank: previous, trial_bank: Some(trial_bank), boots: 0, max_boots: DEFAULT_MAX_BOOTS };
    write_atomic(&record_path(open_dir), record.to_text().as_bytes())
}

/// Records a pending trial for an image that something else installed and
/// selected (`dphone update arm-trial`).  Nothing in flash is changed.
///
/// What the record must say follows from what will happen, not from what
/// is mounted now.  boot2 boots the bank `ctrl` selects, so that bank is
/// the one on trial, and the bank to return to is the other one.  The
/// bank named by `root=` on the kernel command line is evidence for which
/// bank that is only when it differs from the selected one: a system
/// booted from flash runs the bank it would return to.  When `root=` names
/// the selected bank itself there are two readings that nothing here can
/// tell apart: the phone was booted from flash and runs the selected bank
/// (nothing is on trial), or its kernel was started from RAM with the root
/// file system of the bank that is about to get its first boot through
/// boot2 (the other bank is the one to return to).  The same holds when
/// `root=` names neither bank.  Those cases are refused unless the caller
/// names the bank to return to in `return_to`.
///
/// Also refused, because the record could then never do its work: no valid
/// `ctrl` record, a `return_to` equal to the selected bank, either bank not
/// bootable by boot2's rules, and stock firmware in the bank on trial
/// (stock does not count trial boots).
pub fn arm(mtd: &mut dyn Mtd, cmdline: &str, open_dir: &Path, return_to: Option<Bank>) -> Result<TrialRecord> {
    let refused = |m: String| Err(UpdateError::Refused(m));
    let st = crate::inspect::inspect(mtd, cmdline, None)?;
    if !st.ctrl_valid {
        return refused("ctrl holds no valid record, so the bank boot2 boots next is not a recorded choice".into());
    }
    let on_trial = st.next_boot;
    let previous = match (return_to, st.running) {
        (Some(p), _) if p == on_trial => {
            return refused(format!(
                "ctrl selects bank {on_trial:?} for the next boot, so that is the bank that would be on trial;                  it cannot also be the bank to return to (select the bank to try with 'dphone update switch-bank'                  first, which also removes any trial record, then arm the trial)"
            ));
        }
        (Some(p), _) => p,
        (None, Some(r)) if r != on_trial => r,
        (None, Some(r)) => {
            return refused(format!(
                "ctrl selects bank {r:?} for the next boot, and root= on the kernel command line names the same                  bank.  If this system was booted from flash, it runs the selected bank and nothing is on trial:                  select the bank to try with 'dphone update switch-bank' first.  If this kernel was started from                  RAM and bank {r:?} gets its first boot through boot2 next, name the bank to return to:                  'dphone update arm-trial --return-to {}'",
                bank_letter(r.other())
            ));
        }
        (None, None) => {
            return refused(format!(
                "root= on the kernel command line names neither bank, so the bank to return to is not known;                  ctrl selects bank {on_trial:?} for the next boot: if that is the bank to try,                  'dphone update arm-trial --return-to {}'",
                bank_letter(on_trial.other())
            ));
        }
    };
    for (bank, role) in [(on_trial, "on trial"), (previous, "to return to")] {
        let b = &st.banks[bank::index(bank)];
        if b.header.is_none() {
            return refused(format!("bank {bank:?} ({role}) has no valid header in ctrl"));
        }
        if let Err(p) = &b.kernel {
            return refused(format!("bank {bank:?} ({role}) is not bootable: {p}"));
        }
    }
    if let Some(v) = &st.banks[bank::index(on_trial)].version
        && crate::install::is_stock_version(v)
    {
        return refused(format!(
            "bank {on_trial:?} holds stock firmware {v}, which does not count trial boots;              the record would be left for whatever image runs next"
        ));
    }
    let record = TrialRecord {
        previous_bank: previous,
        trial_bank: Some(on_trial),
        boots: 0,
        max_boots: DEFAULT_MAX_BOOTS,
    };
    write_atomic(&record_path(open_dir), record.to_text().as_bytes())?;
    Ok(record)
}

fn bank_letter(b: Bank) -> char {
    match b {
        Bank::A => 'a',
        Bank::B => 'b',
    }
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
