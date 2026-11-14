//! Open DPhone userspace.
//!
//! One executable with several roles, chosen in [`cli`]:
//!
//! - **init role** ([`init`]): PID 1.  Mounts, watchdog, modules, nvdata,
//!   clock, and supervision of the main role.  Synchronous and small.
//! - **main role** ([`mainrole`]): everything else, on one single-threaded
//!   async runtime.  Hosts the SSH server ([`ssh`]) and the diagnostics
//!   socket ([`diag`]).
//! - **diagnostic client** ([`diag::client`]): `dphone <command>` talks to the
//!   running main role over a Unix socket.
//! - **system commands** ([`cmds`]): `mount`, `umount`, `insmod`, `rmmod`,
//!   `lsmod`, `modprobe`, `reboot`, `poweroff`, `halt`, and `dmesg`, which the
//!   image has no other provider for.

pub mod cli;
pub mod clock;
pub mod cmds;
pub mod diag;
pub mod init;
pub mod logging;
pub mod mainrole;
pub mod model;
pub mod paths;
pub mod ssh;
pub mod sys;
pub mod tz;
pub mod update_cli;

/// Crate version, `3.0.0` form.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Product name, shown where nothing parses it.
pub const PRODUCT: &str = "Open DPhone";

/// Version in the stock style (`3_0_0`), for places where stock puts a version.
pub fn stock_version() -> String {
    VERSION.replace('.', "_")
}
