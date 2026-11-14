//! Read-only view of the update state, for `dphone update status` and the
//! web UI.  Nothing here writes flash.

use std::path::Path;

use dphone_hal::Mtd;
use eff::ota::AppHeader;

use crate::bank::{self, Bank};
use crate::boot2::{self, KernelProblem, KernelSpan};
use crate::ctrl::{self, CTRL_PART};
use crate::error::Result;
use crate::flash;
use crate::trial::{self, TrialRecord};

/// One bank as `ctrl` and the flash describe it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BankStatus {
    pub bank: Bank,
    /// The header in `ctrl`, when its CRC is valid (boot2's test).
    pub header: Option<AppHeader>,
    /// Image file name from the header (`2_9_28_D60_firmware.bin`).
    pub image_name: String,
    /// Build date text from the header.
    pub build_date: String,
    /// Version taken from the image name (`2_9_28`), when it has that form.
    pub version: Option<String>,
    /// Whether boot2 could load the kernel stored in this bank.
    pub kernel: std::result::Result<KernelSpan, KernelProblem>,
    pub kernel_bad_blocks: u32,
    pub rootfs_bad_blocks: u32,
}

impl BankStatus {
    /// boot2 would accept this bank: valid header and loadable kernel.
    pub fn bootable(&self) -> bool {
        self.header.is_some() && self.kernel.is_ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// Bank the running system came from (`root=` on the command line).
    pub running: Option<Bank>,
    /// Active side stored in `ctrl` (`None`: no valid record, or a value
    /// other than 0 or 1).
    pub active: Option<Bank>,
    /// Bank boot2 will boot next, by its selection rule.
    pub next_boot: Bank,
    /// A valid record was found.
    pub ctrl_valid: bool,
    /// Copies found (valid first-page copies, good blocks, bad blocks).
    pub ctrl_copies: (u32, u32, u32),
    /// All copies agree.
    pub ctrl_consistent: bool,
    pub banks: [BankStatus; 2],
    /// Pending trial record in nvdata, if any.
    pub trial: Option<TrialRecord>,
}

/// `2_9_28_D60_firmware.bin` -> `2_9_28`.
pub fn version_from_image_name(name: &str) -> Option<String> {
    let (version, _) = name.split_once("_D")?;
    let ok = !version.is_empty() && version.split('_').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    ok.then(|| version.to_string())
}

/// Collects the status.  `open_dir` is `/nvdata/open` (for the trial
/// record); pass `None` when nvdata is not mounted.
pub fn inspect(mtd: &mut dyn Mtd, cmdline: &str, open_dir: Option<&Path>) -> Result<Status> {
    let parts = mtd.partitions()?;
    let scan = ctrl::scan(mtd.open(CTRL_PART)?.as_mut())?;
    let record = scan.effective().copied();
    let mut banks = Vec::with_capacity(2);
    for b in [Bank::A, Bank::B] {
        let header = scan.record.as_ref().and_then(|r| r.sides[bank::index(b)].header());
        let mut kpart = mtd.open(bank::kernel_part(b))?;
        let kernel = boot2::check_stored_kernel(kpart.as_mut())?;
        let (_, kernel_bad_blocks) = flash::capacity(kpart.as_mut())?;
        let (_, rootfs_bad_blocks) = flash::capacity(mtd.open(bank::rootfs_part(b))?.as_mut())?;
        let image_name = header.map(|h| h.name()).unwrap_or_default();
        banks.push(BankStatus {
            bank: b,
            header,
            build_date: header.map(|h| h.date()).unwrap_or_default(),
            version: version_from_image_name(&image_name),
            image_name,
            kernel,
            kernel_bad_blocks,
            rootfs_bad_blocks,
        });
    }
    Ok(Status {
        running: bank::running_bank(cmdline, &parts),
        active: scan.record.as_ref().and_then(|r| bank::from_index(r.active)),
        next_boot: bank::from_index(record.map_or(0, |r| r.boot_side())).unwrap_or(Bank::A),
        ctrl_valid: scan.record.is_some(),
        ctrl_copies: (scan.valid_copies, scan.good_blocks, scan.bad_blocks),
        ctrl_consistent: scan.consistent,
        banks: banks.try_into().expect("two banks"),
        trial: open_dir.and_then(trial::load),
    })
}

#[cfg(test)]
mod tests {
    use super::version_from_image_name;

    #[test]
    fn versions() {
        assert_eq!(version_from_image_name("2_9_28_D60_firmware.bin").as_deref(), Some("2_9_28"));
        assert_eq!(version_from_image_name("3_0_0_D40_firmware.bin").as_deref(), Some("3_0_0"));
        assert_eq!(version_from_image_name("firmware.bin"), None);
        assert_eq!(version_from_image_name("x_D60"), None);
    }
}
