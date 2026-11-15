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
    // The early-boot stage mounts nvdata to read the trial record and hands
    // it over mounted (with the same flags), which saves a second scan.
    let mounted = std::fs::read_to_string("/proc/mounts").is_ok_and(|m| is_mounted(&m));
    let result = if mounted { Ok("the early-boot stage".to_string()) } else { try_mount(flags) };
    match result {
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

/// What a factory reset keeps besides stock's own list
/// (`dphone_config::nvdata::FACTORY_RESET_KEEP`), relative to the nvdata
/// root.
///
/// Stock's `factory-reset.sh` keeps `etc/enet.cfg`, `reboot_reason`,
/// `phoneservice_lockdown`, the two G.729 files, and `if-setup-debug`, and
/// removes every directory but `etc`; so it removes `open/`, and so do we
/// (host keys, authorized keys, the provisioning identity, call log, and the
/// Open DPhone settings file all go, as stock's own equivalents do).  The
/// one addition is the trial-boot record: it is not user data, and a reset
/// that removed it would leave an unconfirmed image installed with nothing
/// to roll it back.  The DSP firmware is in the root file system and is
/// not affected.
pub const FACTORY_RESET_KEEP: &[&str] = &["open/trial"];

/// Perform a factory reset that was requested before the last shutdown
/// (stock's marker file `factory-reset`), before anything reads nvdata.
/// Returns whether one ran.  On an error the marker stays, so the next boot
/// tries again.
pub fn run_pending_factory_reset(root: &std::path::Path) -> bool {
    let nv = dphone_config::Nvdata::new(root);
    match dphone_config::ConfigStore::run_pending_factory_reset(&nv, FACTORY_RESET_KEEP) {
        Ok(false) => false,
        Ok(true) => {
            sys::sync();
            warn!("factory reset performed");
            true
        }
        Err(e) => {
            error!("factory reset: {e}");
            false
        }
    }
}

/// Whether `/proc/mounts` text shows a JFFS2 volume on the nvdata mount
/// point.
fn is_mounted(proc_mounts: &str) -> bool {
    proc_mounts.lines().any(|line| {
        let mut f = line.split_ascii_whitespace();
        f.nth(1) == Some(NVDATA) && f.next() == Some("jffs2")
    })
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
    fn sees_nvdata_handed_over_mounted() {
        assert!(is_mounted("/dev/root / jffs2 ro,relatime 0 0\nmtd:nvdata /nvdata jffs2 rw,nosuid,nodev,noatime 0 0\n"));
        assert!(!is_mounted("/dev/root / jffs2 ro,relatime 0 0\nproc /proc proc rw 0 0\n"));
        assert!(!is_mounted("tmpfs /nvdata tmpfs rw 0 0\n"));
    }

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

    #[test]
    fn factory_reset_runs_once_and_keeps_the_trial_record() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let put = |rel: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        for f in [
            "etc/enet.cfg",
            "etc/.passwd",
            "ui-config.xml",
            "fetched-config.xml",
            "dphone_settings.txtpb",
            "root/.ssh/authorized_keys",
            "open/trial",
            "open/ssh_host_ed25519_key",
            "open/kv/calllog.json",
            "ringtones/Mine.sln16",
            "certs/certs.crt",
        ] {
            put(f);
        }
        // Not requested: nothing happens.
        assert!(!run_pending_factory_reset(root));
        assert!(root.join("ui-config.xml").exists());

        put("factory-reset");
        assert!(run_pending_factory_reset(root));
        for kept in ["etc/enet.cfg", "open/trial"] {
            assert!(root.join(kept).exists(), "{kept} must survive");
        }
        for gone in [
            "etc/.passwd",
            "ui-config.xml",
            "fetched-config.xml",
            "dphone_settings.txtpb",
            "root/.ssh/authorized_keys",
            "open/ssh_host_ed25519_key",
            "open/kv/calllog.json",
            "ringtones/Mine.sln16",
            "certs/certs.crt",
            "factory-reset",
        ] {
            assert!(!root.join(gone).exists(), "{gone} must be removed");
        }
        // The marker is gone, so the next boot does not reset again.
        put("ui-config.xml");
        assert!(!run_pending_factory_reset(root));
        assert!(root.join("ui-config.xml").exists());
    }
}
