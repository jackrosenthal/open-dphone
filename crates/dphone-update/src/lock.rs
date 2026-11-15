//! One update at a time.
//!
//! Everything that rewrites a bank or `ctrl` (an installation from the
//! command line, the web page, or provisioning; a bank switch; giving up a
//! trial) first takes an exclusive `flock` on the directory that holds the
//! trial record (`/nvdata/open`).  The lock is on the directory itself:
//! nothing is created, so a full or missing `/tmp` does not matter, and the
//! kernel drops it when the holder exits, however it exits.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;

use crate::error::{Result, UpdateError};

/// Held for as long as the value lives.
#[derive(Debug)]
pub struct UpdateLock(#[allow(dead_code)] File);

/// Takes the lock, or refuses at once when another update holds it.
pub fn acquire(open_dir: &Path) -> Result<UpdateLock> {
    let _ = std::fs::create_dir_all(open_dir);
    let dir = File::open(open_dir).map_err(|e| {
        UpdateError::Io(std::io::Error::new(e.kind(), format!("{} (for the update lock): {e}", open_dir.display())))
    })?;
    // SAFETY: `dir` is an open descriptor for the whole call.
    if unsafe { libc::flock(dir.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(UpdateLock(dir));
    }
    let e = std::io::Error::last_os_error();
    if e.kind() == std::io::ErrorKind::WouldBlock {
        Err(UpdateError::Refused(
            "another firmware update or bank switch is running; wait for it to finish ('dphone update status' shows the banks)".into(),
        ))
    } else {
        Err(UpdateError::Io(e))
    }
}
