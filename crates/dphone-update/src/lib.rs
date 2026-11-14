//! On-device firmware update for Open DPhone (plan section 12): install an
//! EFF into the spare bank, maintain boot2's `ctrl` record, and support
//! trial boots with rollback.
//!
//! - [`install::Updater`]: stream an EFF (ours or stock) into the bank that
//!   is not running, verify by reading back, and commit.
//! - [`ctrl`]: the `ctrl` partition's replication layout per family and a
//!   power-cut-tolerant writer.
//! - [`trial`]: the pending-trial record in nvdata and the rollback the
//!   init role calls.
//! - [`inspect`]: read-only status.
//! - [`boot2`]: a model of the stock second-stage loader's decisions, used
//!   to refuse images and states it would not boot.
//!
//! Never touched: `boot1`, `boot2`, the partition table, `nvdata` contents
//! other than `/nvdata/open/trial`.  Bootstrap (type 1) OTAs are rejected
//! by the OTA parser.
//!
//! Typical use by the main role:
//!
//! ```ignore
//! let installed = Updater::new(hal.mtd.as_mut(), model, &cmdline)
//!     .store(store.clone())
//!     .install_eff(upload)?;
//! trial::begin(&open_dir, &installed)?;
//! // then reboot
//! ```
//!
//! and by the init role's rollback hook (`fn(&TrialRecord) -> io::Result<()>`):
//!
//! ```ignore
//! trial::rollback(mtd, model, &cmdline, previous_bank).map_err(Into::into)
//! ```

pub mod bank;
pub mod boot2;
pub mod ctrl;
pub mod error;
mod flash;
pub mod inspect;
pub mod install;
pub mod trial;

#[cfg(test)]
mod memnand;
#[cfg(test)]
mod tests;

pub use bank::Bank;
pub use ctrl::Replication;
pub use error::{Result, UpdateError};
pub use inspect::{BankStatus, Status, inspect};
pub use install::{Installed, Options, Updater, is_stock_version, select_bank};
