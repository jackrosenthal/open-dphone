//! The flash side of a provisioned firmware update, over
//! `opendeskphone-update`.

use crate::firmware::{FirmwareInstaller, ImageSum, InstallError};
use opendeskphone_hal::Mtd;
use opendeskphone_state::{Model, Store};
use opendeskphone_update::{UpdateError, Updater, bank, ctrl};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Installs through the updater into the bank that is not running, and
/// starts the trial boot.
pub struct MtdInstaller {
    pub mtd: Arc<Mutex<Box<dyn Mtd>>>,
    pub model: Model,
    /// The kernel command line (`/proc/cmdline`): which bank is running.
    pub cmdline: String,
    /// `/nvdata/opendeskphone`, where the trial-boot record lives.
    pub own_dir: PathBuf,
    pub store: Store,
}

impl FirmwareInstaller for MtdInstaller {
    /// The running bank's header in `ctrl`.  That record is CRC-protected
    /// and is what boot2 itself goes by; this updater writes it only after
    /// reading the whole image back from flash, so it is trusted without
    /// reading 45 MiB of flash at every check.
    fn running_image(&self) -> Result<ImageSum, String> {
        let mut mtd = self.mtd.lock().map_err(|_| "flash lock poisoned".to_string())?;
        let parts = mtd.partitions().map_err(|e| e.to_string())?;
        let running = bank::running_bank(&self.cmdline, &parts).ok_or("not running from either firmware bank")?;
        let mut part = mtd.open(ctrl::CTRL_PART).map_err(|e| e.to_string())?;
        let record = ctrl::read(part.as_mut()).map_err(|e| e.to_string())?;
        let header = record.sides[running.index() as usize]
            .header()
            .ok_or("the running bank has no valid header in ctrl")?;
        Ok(ImageSum::of(&header))
    }

    fn install(&self, image: &mut dyn Read) -> Result<(), InstallError> {
        let mut mtd = self.mtd.lock().map_err(|_| InstallError::Failed("flash lock poisoned".into()))?;
        // The download goes straight into the spare bank, one erase block
        // at a time; the updater records the trial when it selects the bank.
        let installed = Updater::new(mtd.as_mut(), self.model, &self.cmdline, &self.own_dir)
            .store(self.store.clone())
            .install_eff(image)
            .map_err(|e| match e {
                UpdateError::WrongModel { .. } => InstallError::Refused(e.to_string()),
                other => InstallError::Failed(other.to_string()),
            })?;
        log::info!("update: installed {} into bank {:?}", installed.version, installed.bank);
        Ok(())
    }
}
