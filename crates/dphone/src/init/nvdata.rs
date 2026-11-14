//! Mounting nvdata.  The partition is found by name, never by number or
//! offset (the layout comes from boot2's `mtdparts=`).

use std::io;

use log::{error, info, warn};

use crate::paths::NVDATA;
use crate::sys;

/// Index of the MTD partition called `name`, from `/proc/mtd` text.
pub fn mtd_index(proc_mtd: &str, name: &str) -> Option<u32> {
    let quoted = format!("\"{name}\"");
    proc_mtd.lines().find_map(|line| {
        let (dev, rest) = line.split_once(':')?;
        if rest.trim_end().ends_with(&quoted) { dev.strip_prefix("mtd")?.parse().ok() } else { None }
    })
}

/// Outcome of the nvdata mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NvdataState {
    Mounted,
    /// The real volume could not be mounted.  A tmpfs stands in so the main
    /// role can run its recovery screen and web page; nothing persists.
    /// Unlike stock, the partition is NOT erased.
    Recovery,
}

/// Mount nvdata read-write at `/nvdata`.
pub fn mount() -> NvdataState {
    let flags = libc::MS_NOATIME | libc::MS_NOSUID | libc::MS_NODEV;
    match try_mount(flags) {
        Ok(source) => {
            info!("nvdata mounted from {source}");
            if let Err(e) = std::fs::create_dir_all(format!("{NVDATA}/open")) {
                warn!("mkdir {NVDATA}/open: {e}");
            }
            NvdataState::Mounted
        }
        Err(e) => {
            error!("nvdata mount failed: {e}; entering recovery (partition left untouched)");
            if let Err(e) = sys::mount("tmpfs", NVDATA, "tmpfs", flags, "mode=0755,size=4m") {
                error!("recovery tmpfs on {NVDATA}: {e}");
            }
            let _ = std::fs::create_dir_all(format!("{NVDATA}/open"));
            let _ = std::fs::write(format!("{}/recovery", crate::paths::RUN_DIR), b"nvdata\n");
            NvdataState::Recovery
        }
    }
}

fn try_mount(flags: libc::c_ulong) -> io::Result<String> {
    let proc_mtd = std::fs::read_to_string("/proc/mtd")?;
    let index = mtd_index(&proc_mtd, "nvdata")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no MTD partition named nvdata"))?;
    // JFFS2 accepts the block device or the MTD name; try both so the kernel
    // does not need MTD_BLOCK just for this.
    let candidates = [format!("/dev/mtdblock{index}"), format!("mtd{index}")];
    let mut last = io::Error::other("no candidate");
    for source in candidates {
        match sys::mount(&source, NVDATA, "jffs2", flags, "") {
            Ok(()) => return Ok(source),
            Err(e) => {
                warn!("mount {source} on {NVDATA}: {e}");
                last = e;
            }
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_partition() {
        let text = "dev:    size   erasesize  name\n\
                    mtd0: 00020000 00020000 \"boot1\"\n\
                    mtd2: 00080000 00020000 \"ctrl\"\n\
                    mtd3: 00ee0000 00020000 \"nvdata\"\n\
                    mtd8: 01000000 00020000 \"userdata\"\n";
        assert_eq!(mtd_index(text, "nvdata"), Some(3));
        assert_eq!(mtd_index(text, "ctrl"), Some(2));
        assert_eq!(mtd_index(text, "data"), None);
    }
}
