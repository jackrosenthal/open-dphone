//! Early mounts.  The root filesystem is read-only JFFS2, so every writable
//! path is a tmpfs or nvdata.

use std::io;

use log::{info, warn};

use crate::sys;

struct Mount {
    source: &'static str,
    target: &'static str,
    fstype: &'static str,
    flags: libc::c_ulong,
    data: &'static str,
    /// Skip quietly when the mount point is missing from the rootfs.
    optional: bool,
}

const NSE: libc::c_ulong = libc::MS_NOSUID | libc::MS_NOEXEC;

const EARLY: &[Mount] = &[
    Mount { source: "proc", target: "/proc", fstype: "proc", flags: NSE | libc::MS_NODEV, data: "", optional: false },
    Mount { source: "sysfs", target: "/sys", fstype: "sysfs", flags: NSE | libc::MS_NODEV, data: "", optional: false },
    Mount { source: "devtmpfs", target: "/dev", fstype: "devtmpfs", flags: libc::MS_NOSUID, data: "mode=0755", optional: false },
    Mount { source: "devpts", target: "/dev/pts", fstype: "devpts", flags: NSE, data: "gid=5,mode=620,ptmxmode=666", optional: false },
    // /tmp holds pushed builds (`/tmp/dphone`), so it is executable and roomy.
    Mount { source: "tmpfs", target: "/tmp", fstype: "tmpfs", flags: libc::MS_NOSUID | libc::MS_NODEV, data: "mode=1777,size=40m", optional: false },
    Mount { source: "tmpfs", target: "/run", fstype: "tmpfs", flags: NSE | libc::MS_NODEV, data: "mode=0755,size=4m", optional: false },
    Mount { source: "tmpfs", target: "/var", fstype: "tmpfs", flags: NSE | libc::MS_NODEV, data: "mode=0755,size=4m", optional: true },
    // Kernel crash records (ramoops), when the kernel has them.
    Mount { source: "pstore", target: "/sys/fs/pstore", fstype: "pstore", flags: NSE | libc::MS_NODEV, data: "", optional: true },
];

/// Mount everything in the early table.  Failures are logged and skipped:
/// a missing `/sys` is bad, but not worse than a PID 1 that gives up.
pub fn mount_early() {
    for m in EARLY {
        if m.target == "/dev/pts" {
            let _ = std::fs::create_dir("/dev/pts");
        }
        match sys::mount(m.source, m.target, m.fstype, m.flags, m.data) {
            Ok(()) => {}
            // Already mounted (e.g., devtmpfs auto-mounted by the kernel).
            Err(e) if e.raw_os_error() == Some(libc::EBUSY) => {}
            // No mount point in this rootfs, or no such filesystem in this kernel.
            Err(e) if m.optional && (e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(libc::ENODEV)) => {}
            Err(e) => warn!("mount {} on {}: {e}", m.fstype, m.target),
        }
    }
    {
        let dir = crate::paths::RUN_DIR;
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!("mkdir {dir}: {e}");
        }
    }
    info!("early mounts done");
}

/// Unmount what holds persistent data, before the reboot system call.
pub fn unmount_for_shutdown() {
    sys::sync();
    if let Err(e) = sys::umount(crate::paths::NVDATA, 0)
        && !matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOENT)) {
            warn!("umount {}: {e}; trying lazy", crate::paths::NVDATA);
            let _ = sys::umount(crate::paths::NVDATA, libc::MNT_DETACH);
        }
    sys::sync();
}
