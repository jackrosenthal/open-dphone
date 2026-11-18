//! Hardware watchdog.  boot2 hands off with it disabled, so the init role
//! arms it early and is the only feeder.

use std::fs::File;
use std::io::Write;

use log::{info, warn};

use crate::sys;

const WDIOC_KEEPALIVE: libc::c_ulong = 0x8004_5705;
const WDIOC_SETTIMEOUT: libc::c_ulong = 0xc004_5706;

/// Seconds without a feed before the hardware resets the phone.
pub const TIMEOUT_SECS: libc::c_int = 30;

/// The timeout set just before jumping into a kernel loaded with `dphone
/// ramboot`.  It has to cover the copy of the image into place (some
/// 20 MiB, with the caches off), the decompressor, and the new kernel up
/// to its watchdog driver, which then keeps the watchdog fed until its
/// init role opens it.  60 s is also what the stock firmware runs the
/// watchdog with, and with which every RAM boot out of stock was made.
pub const KEXEC_TIMEOUT_SECS: libc::c_int = 60;

pub struct Watchdog {
    dev: Option<File>,
    warned: bool,
    disabled: bool,
}

impl Watchdog {
    /// Open and arm `/dev/watchdog`.  When the device is missing (driver not
    /// loaded yet) the handle stays empty and [`Watchdog::feed`] retries.
    pub fn open() -> Self {
        let mut w = Watchdog { dev: None, warned: false, disabled: false };
        w.try_open();
        w
    }

    fn try_open(&mut self) {
        match std::fs::OpenOptions::new().write(true).open("/dev/watchdog") {
            Ok(f) => {
                let mut timeout = TIMEOUT_SECS;
                match sys::ioctl_int(&f, WDIOC_SETTIMEOUT, &mut timeout) {
                    Ok(()) => info!("watchdog armed, timeout {timeout} s"),
                    Err(e) => warn!("watchdog armed, could not set timeout: {e}"),
                }
                self.dev = Some(f);
            }
            Err(e) => {
                if !self.warned {
                    warn!("/dev/watchdog: {e}");
                    self.warned = true;
                }
            }
        }
    }

    /// A handle that never opens the device (host test mode).
    pub fn never() -> Self {
        Watchdog { dev: None, warned: true, disabled: true }
    }

    /// Try to stop the watchdog (magic close).  Works only when the driver
    /// allows it; otherwise the watchdog stays armed.
    pub fn disarm(&mut self) {
        if let Some(mut f) = self.dev.take() {
            let _ = f.write_all(b"V");
        }
        self.disabled = true;
    }

    /// Stop feeding without disarming: the hardware resets the phone when
    /// the timeout runs out.  This is the reset path that has been seen to
    /// work on the hardware (the kernel's restart code has not).
    pub fn starve(&mut self) {
        self.disabled = true;
    }

    /// Change the timeout of the armed watchdog and feed it.  Returns
    /// whether the driver took it.
    pub fn set_timeout(&mut self, secs: libc::c_int) -> bool {
        let Some(f) = self.dev.as_ref() else { return false };
        let mut timeout = secs;
        sys::ioctl_int(f, WDIOC_SETTIMEOUT, &mut timeout).is_ok()
    }

    pub fn is_armed(&self) -> bool {
        self.dev.is_some()
    }

    pub fn feed(&mut self) {
        if self.disabled {
            return;
        }
        if self.dev.is_none() {
            self.try_open();
        }
        if let Some(f) = self.dev.as_mut() {
            let mut dummy: libc::c_int = 0;
            if sys::ioctl_int(f, WDIOC_KEEPALIVE, &mut dummy).is_err() {
                let _ = f.write_all(b"\0");
            }
        }
    }
}
