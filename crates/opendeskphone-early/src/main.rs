//! The early-boot stage: `/init` of the initramfs built into the kernel
//! image, so it lives in the kernel partition and runs before the root
//! file system is needed.
//!
//! boot2 boots whatever `ctrl` selects and never falls back, and the init
//! role that used to count trial boots is in the root file system: an
//! image whose root does not mount, or mounts and has no init to run,
//! reset forever.  This program is the part of the fallback that does not
//! depend on the new root:
//!
//! 1. Find the bank that was booted (`root=/dev/mtdblockN` naming `root0`
//!    or `root1`).  A system started from RAM (`dphone ramboot`) or with a
//!    test root is not a bank: skip to 4.
//! 2. Mount nvdata and read the trial record.  Nothing on trial in this
//!    boot: go to 4 (nvdata stays mounted and is handed to the init role).
//! 3. Count the boot in the record.  When the trial's boots are used up,
//!    select the previous bank in `ctrl` (the updater's own writer,
//!    `opendeskphone-ctrl`), remove the record, and restart.
//! 4. Mount the root named on the command line and run its init, with
//!    `DPHONE_EARLY=1` in the environment so the init role does not count
//!    the boot a second time.  When that fails during a trial (no mount,
//!    no init, init cannot be executed), go back as in 3 at once.  With no
//!    trial there is nothing to go back to: restart after a pause, as the
//!    kernel's panic would.
//!
//! After the hand-over this program is gone.  A root whose init starts and
//! then dies, a kernel panic, or a watchdog reset are covered by 3: every
//! such boot was counted before the root was touched.  The init role still
//! confirms the trial (removes the record) when the main role is healthy.
//!
//! No standard library, no allocation, raw system calls (the C library
//! gives only the start-up code and `memcpy` and its kin): the whole program
//! is a few kilobytes of the 2 MiB kernel slot.  It is built for the phone
//! only; on other targets the crate is an empty program so the workspace
//! builds, and what it decides is tested in `opendeskphone-ctrl`.

#![cfg_attr(target_arch = "arm", no_std, no_main)]

#[cfg(target_arch = "arm")]
mod arm;

#[cfg(not(target_arch = "arm"))]
fn main() {
    eprintln!("opendeskphone-early runs only as /init of the phone's kernel");
    std::process::exit(1);
}
