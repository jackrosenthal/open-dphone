//! Installing an EFF into the bank that is not running.
//!
//! The order of operations, and the state boot2 sees after each:
//!
//! | Step | Flash state if power is lost right after |
//! | --- | --- |
//! | 1. Checks (model, OTA header CRCs, sizes, kernel header, the kept bank is bootable) | untouched |
//! | 2. `ctrl`: target header invalidated, active side set to the kept bank | boots the kept bank |
//! | 3. Erase and write kernel, then root file system; each block read back | boots the kept bank |
//! | 4. Stream CRCs, end of stream, whole-image read-back CRCs | boots the kept bank |
//! | 5. `ctrl`: target header filled in, active side set to the target | boots the new image |
//!
//! Steps 2 and 5 rewrite `ctrl` one block at a time ([`crate::ctrl::write`]),
//! so a power cut inside either leaves boot2 with the record from before or
//! after the step, both of which are in the table.
//!
//! The image arrives as a stream and is never held in memory: one erase
//! block (128 KiB) of data plus one of read-back at a time.  The price,
//! as with stock, is that a stream which turns out truncated or corrupt
//! has already cost the spare bank its old contents; the running bank is
//! not affected and the spare stays marked invalid.

use std::io::Read;

use dphone_hal::Mtd;
use dphone_state::{Model, Store, Update, UpdatePhase};
use eff::ctrl::Side;
use eff::ota::{AppHeader, OtaReader};
use eff::EffReader;

use crate::bank::{self, Bank};
use crate::boot2;
use crate::ctrl::{self, CTRL_PART, Replication};
use crate::error::{Result, UpdateError};
use crate::flash::{self, OnBlockFailure};

/// Knobs; the defaults are right for the phone.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Bank to write.  Default: the one not running; when the running
    /// system is from neither bank (RAM boot), the one boot2 would not
    /// boot next.  Naming the running bank is refused.
    pub target: Option<Bank>,
    /// Mark a block bad when it fails to erase, program, or read back,
    /// and continue in the next block (stock's behavior on a failed
    /// write).  Off by default: bad-block marks are permanent and shrink
    /// the 2 MiB kernel slot, so a failure stops the update instead and
    /// the cause can be looked at.
    pub mark_bad_on_failure: bool,
    /// Go ahead even if the bank that stays is not bootable by boot2's
    /// rules (for recovery work from a RAM boot).
    pub allow_unbootable_other: bool,
}

/// A finished, committed installation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// The bank now selected for the next boot.
    pub bank: Bank,
    /// The bank that was kept; the rollback target.
    pub previous: Bank,
    /// Version from the EFF's encrypted header.
    pub version: String,
    /// The image is stock firmware (version below 3).
    pub is_stock: bool,
    /// The header now stored in `ctrl` for `bank`.
    pub header: AppHeader,
    /// Bad blocks skipped in the kernel and root partitions.
    pub bad_blocks: (u32, u32),
}

/// Whether a version string (`2_9_28`, `3_0_0`, `3.0.0`) is a stock
/// release: stock stopped at 2.x, Open DPhone starts at 3.
pub fn is_stock_version(version: &str) -> bool {
    let major: String = version.trim().chars().take_while(char::is_ascii_digit).collect();
    major.parse::<u32>().map_or(true, |m| m < 3)
}

type ProgressFn<'a> = Box<dyn FnMut(&UpdatePhase) + 'a>;

/// The updater.  One per installation.
pub struct Updater<'a> {
    mtd: &'a mut dyn Mtd,
    model: Model,
    cmdline: String,
    options: Options,
    store: Option<Store>,
    progress: Option<ProgressFn<'a>>,
    last: UpdatePhase,
}

impl<'a> Updater<'a> {
    /// `cmdline` is the kernel command line (`/proc/cmdline`), from which
    /// the running bank is taken.
    pub fn new(mtd: &'a mut dyn Mtd, model: Model, cmdline: &str) -> Self {
        Updater {
            mtd,
            model,
            cmdline: cmdline.to_string(),
            options: Options::default(),
            store: None,
            progress: None,
            last: UpdatePhase::Idle,
        }
    }

    pub fn options(mut self, options: Options) -> Self {
        self.options = options;
        self
    }

    /// Publish progress in the state store's `Update` topic.
    pub fn store(mut self, store: Store) -> Self {
        self.store = Some(store);
        self
    }

    /// Also call this on every progress change.
    pub fn progress(mut self, f: impl FnMut(&UpdatePhase) + 'a) -> Self {
        self.progress = Some(Box::new(f));
        self
    }

    fn phase(&mut self, phase: UpdatePhase) {
        if phase == self.last {
            return;
        }
        self.last = phase.clone();
        if let Some(f) = &mut self.progress {
            f(&phase);
        }
        if let Some(s) = &self.store {
            s.update::<Update, _>(|u| u.phase = phase);
        }
    }

    /// Installs an EFF read from `reader` (our images and stock images
    /// alike), committing it as the next boot.  On error nothing is
    /// committed: the next boot is the bank that was kept.
    pub fn install_eff<R: Read>(&mut self, reader: R) -> Result<Installed> {
        let result = self.install_eff_inner(reader);
        match &result {
            Ok(_) => self.phase(UpdatePhase::Complete),
            Err(e) => {
                log::error!("firmware update failed: {e}");
                self.phase(UpdatePhase::Failed(e.to_string()));
            }
        }
        result
    }

    fn install_eff_inner<R: Read>(&mut self, reader: R) -> Result<Installed> {
        self.phase(UpdatePhase::Checking);
        let eff = EffReader::new(reader)?;
        let header = eff.header().clone();
        let phone = self.model.to_string();
        if !header.model.trim().eq_ignore_ascii_case(&phone) {
            return Err(UpdateError::WrongModel { image: header.model, phone });
        }
        let is_stock = is_stock_version(&header.version);
        if let Some(s) = &self.store {
            s.update::<Update, _>(|u| {
                u.incoming_version = header.version.clone();
                u.incoming_is_stock = is_stock;
            });
        }
        log::info!(
            "firmware update: {} {}{}",
            header.model,
            header.version,
            if is_stock { " (stock firmware)" } else { "" }
        );
        self.install_ota(eff, &header.version, is_stock)
    }

    /// Installs a raw OTA stream (already unwrapped and model-checked by
    /// the caller).
    pub fn install_ota<R: Read>(&mut self, reader: R, version: &str, is_stock: bool) -> Result<Installed> {
        self.phase(UpdatePhase::Checking);
        // The 160-byte header: magic, type 0, and both header CRCs.
        let mut ota = OtaReader::new(reader)?;
        let app = ota.header().app;
        if app.kernel_size == 0 || app.rootfs_size == 0 {
            return Err(UpdateError::Image("empty kernel or root file system".into()));
        }

        // Where things are, by name; nothing here knows an offset.
        let parts = self.mtd.partitions()?;
        let running = bank::running_bank(&self.cmdline, &parts);
        let mut ctrl_part = self.mtd.open(CTRL_PART)?;
        let repl = Replication::for_family(self.model.family());
        let mut rec = ctrl::read(ctrl_part.as_mut())?;
        let target = match (self.options.target, running) {
            (Some(t), Some(r)) if t == r => {
                return Err(UpdateError::Refused(format!("bank {t:?} holds the running system")));
            }
            (Some(t), _) => t,
            (None, Some(r)) => r.other(),
            (None, None) => bank::from_index(rec.boot_side()).unwrap_or(Bank::A).other(),
        };
        let keep = target.other();
        let (ti, ki) = (bank::index(target), bank::index(keep));
        let mut kpart = self.mtd.open(bank::kernel_part(target))?;
        let mut rpart = self.mtd.open(bank::rootfs_part(target))?;

        // The bank that stays must be one boot2 will boot once the target
        // is invalid: header valid and a loadable kernel.
        if !self.options.allow_unbootable_other {
            if !rec.sides[ki].is_valid() {
                return Err(UpdateError::Refused(format!(
                    "bank {keep:?} has no valid header in ctrl; updating would leave no bootable bank"
                )));
            }
            let mut other_k = self.mtd.open(bank::kernel_part(keep))?;
            if let Err(p) = boot2::check_stored_kernel(other_k.as_mut())? {
                return Err(UpdateError::Refused(format!("bank {keep:?} is not bootable: {p}")));
            }
        }

        // Sizes against what the partitions can hold after bad blocks.
        let (kcap, kbad) = flash::capacity(kpart.as_mut())?;
        let (rcap, rbad) = flash::capacity(rpart.as_mut())?;
        if app.kernel_size as u64 > kcap {
            return Err(UpdateError::Layout(format!(
                "kernel is {} bytes; {} holds {kcap} ({kbad} bad blocks)",
                app.kernel_size,
                kpart.info().name
            )));
        }
        if app.rootfs_size as u64 > rcap {
            return Err(UpdateError::Layout(format!(
                "root file system is {} bytes; {} holds {rcap} ({rbad} bad blocks)",
                app.rootfs_size,
                rpart.info().name
            )));
        }

        // The start of the kernel, before anything is touched: boot2 loads
        // by the image's own header and hangs or overwrites itself on a
        // bad one, and no CRC would catch that.
        let mut kernel = ota.next_component().expect("kernel component");
        let head_len = (app.kernel_size as usize).min(kpart.info().erase_size as usize);
        let mut head = vec![0u8; head_len];
        kernel.read_exact(&mut head)?;
        boot2::check_kernel_header(&head, app.kernel_size as u64)
            .map_err(|p| UpdateError::Image(p.to_string()))?;

        let on_failure =
            if self.options.mark_bad_on_failure { OnBlockFailure::MarkBad } else { OnBlockFailure::Fail };

        // Step 2: from here on the target is invalid until committed.
        rec.sides[ti] = Side::INVALID;
        rec.active = ki as u32;
        ctrl::write(ctrl_part.as_mut(), &rec, repl)?;
        let now = ctrl::read(ctrl_part.as_mut())?;
        if now.sides[ti].is_valid() || (now.sides[ki].is_valid() && now.boot_side() != ki as u32) {
            return Err(UpdateError::Ctrl("target bank is still selectable after invalidation".into()));
        }

        // Step 3: kernel.
        self.phase(UpdatePhase::Erasing { percent: 0 });
        flash::erase_all(kpart.as_mut(), on_failure, |_, _| {})?;
        self.phase(UpdatePhase::WritingKernel { percent: 0 });
        let kskipped = flash::program(kpart.as_mut(), &head, &mut kernel, app.kernel_size as u64, on_failure, |d, t| {
            self.phase(UpdatePhase::WritingKernel { percent: percent(d, t) })
        })?;
        drop(head);
        kernel.finish()?;

        // Root file system.
        let mut rootfs = ota.next_component().expect("rootfs component");
        flash::erase_all(rpart.as_mut(), on_failure, |d, t| {
            self.phase(UpdatePhase::Erasing { percent: percent(d, t) })
        })?;
        self.phase(UpdatePhase::WritingRootfs { percent: 0 });
        let rskipped = flash::program(rpart.as_mut(), &[], &mut rootfs, app.rootfs_size as u64, on_failure, |d, t| {
            self.phase(UpdatePhase::WritingRootfs { percent: percent(d, t) })
        })?;
        rootfs.finish()?;

        // Step 4: the stream must end here (this also checks the EFF's
        // final padding), and the flash must read back as the image.
        ota.finish()?;
        self.phase(UpdatePhase::Verifying { percent: 0 });
        let total = app.kernel_size as u64 + app.rootfs_size as u64;
        let kcrc = flash::readback_crc(kpart.as_mut(), app.kernel_size as u64, true, |d, _| {
            self.phase(UpdatePhase::Verifying { percent: percent(d, total) })
        })?;
        if kcrc != app.kernel_crc {
            return Err(UpdateError::Verify(format!("kernel reads back with CRC {kcrc:#010x}")));
        }
        let rcrc = flash::readback_crc(rpart.as_mut(), app.rootfs_size as u64, true, |d, _| {
            self.phase(UpdatePhase::Verifying { percent: percent(app.kernel_size as u64 + d, total) })
        })?;
        if rcrc != app.rootfs_crc {
            return Err(UpdateError::Verify(format!("root file system reads back with CRC {rcrc:#010x}")));
        }
        if let Err(p) = boot2::check_stored_kernel(kpart.as_mut())? {
            return Err(UpdateError::Verify(format!("stored kernel: {p}")));
        }

        // Step 5: commit.
        self.phase(UpdatePhase::Committing);
        rec.sides[ti] = Side::from_header(&app);
        rec.active = ti as u32;
        ctrl::write(ctrl_part.as_mut(), &rec, repl)?;
        let now = ctrl::read(ctrl_part.as_mut())?;
        if now != rec || now.boot_side() != ti as u32 {
            return Err(UpdateError::Ctrl("committed record does not read back".into()));
        }
        log::info!("firmware update: bank {target:?} written and selected ({version})");
        Ok(Installed {
            bank: target,
            previous: keep,
            version: version.to_string(),
            is_stock,
            header: app,
            bad_blocks: (kbad.max(kskipped), rbad.max(rskipped)),
        })
    }
}

fn percent(done: u64, total: u64) -> u8 {
    (done.min(total) * 100).checked_div(total).unwrap_or(100) as u8
}

/// Makes `bank` the next boot without writing any image: sets the active
/// side in `ctrl`.  Refused unless boot2 would then really boot it (valid
/// header and a loadable kernel).  This is the rollback primitive.
pub fn select_bank(mtd: &mut dyn Mtd, model: Model, bank: Bank) -> Result<()> {
    let mut ctrl_part = mtd.open(CTRL_PART)?;
    let mut rec = ctrl::read(ctrl_part.as_mut())?;
    let i = bank::index(bank);
    if !rec.sides[i].is_valid() {
        return Err(UpdateError::Refused(format!("bank {bank:?} has no valid header in ctrl")));
    }
    if let Err(p) = boot2::check_stored_kernel(mtd.open(bank::kernel_part(bank))?.as_mut())? {
        return Err(UpdateError::Refused(format!("bank {bank:?} is not bootable: {p}")));
    }
    rec.active = i as u32;
    ctrl::write(ctrl_part.as_mut(), &rec, Replication::for_family(model.family()))?;
    let now = ctrl::read(ctrl_part.as_mut())?;
    if now.boot_side() != i as u32 {
        return Err(UpdateError::Ctrl("bank selection does not read back".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_stock_version;

    #[test]
    fn stock_versions() {
        assert!(is_stock_version("2_9_28"));
        assert!(is_stock_version("1_4_0"));
        assert!(is_stock_version("2.9.28"));
        assert!(!is_stock_version("3_0_0"));
        assert!(!is_stock_version("3.1.0"));
        assert!(!is_stock_version("10_0_0"));
        // Unparseable: treat as foreign.
        assert!(is_stock_version("custom"));
    }
}
