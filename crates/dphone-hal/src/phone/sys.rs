//! ioctl plumbing shared by the phone backend.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub const IOC_NONE: u32 = 0;
pub const IOC_WRITE: u32 = 1;
pub const IOC_READ: u32 = 2;

/// The generic Linux `_IOC` encoding (ARM and x86 use the same one).
pub const fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr as u32
}
pub const fn io(ty: u8, nr: u8) -> u32 {
    ioc(IOC_NONE, ty, nr, 0)
}
pub const fn ior<T>(ty: u8, nr: u8) -> u32 {
    ioc(IOC_READ, ty, nr, size_of::<T>())
}
pub const fn iow<T>(ty: u8, nr: u8) -> u32 {
    ioc(IOC_WRITE, ty, nr, size_of::<T>())
}
pub const fn iowr<T>(ty: u8, nr: u8) -> u32 {
    ioc(IOC_READ | IOC_WRITE, ty, nr, size_of::<T>())
}

/// `ioctl(fd, req, arg)`, retried on `EINTR`.
pub fn ioctl<T>(fd: RawFd, req: u32, arg: &mut T) -> io::Result<i32> {
    loop {
        // SAFETY: `arg` is a valid, exclusively borrowed `T`, and every
        // caller pairs `req` with the struct type the kernel expects.
        let r = unsafe { libc::ioctl(fd, req as libc::Ioctl, arg as *mut T) };
        if r >= 0 {
            return Ok(r);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// An ioctl with no argument.
pub fn ioctl_none(fd: RawFd, req: u32) -> io::Result<i32> {
    loop {
        // SAFETY: no pointer argument.
        let r = unsafe { libc::ioctl(fd, req as libc::Ioctl, 0usize) };
        if r >= 0 {
            return Ok(r);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

pub fn open_rw(path: &Path, extra_flags: i32) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).custom_flags(extra_flags).open(path)
}

pub fn open_ro(path: &Path, extra_flags: i32) -> io::Result<File> {
    OpenOptions::new().read(true).custom_flags(extra_flags).open(path)
}

/// Clears `O_NONBLOCK` on a descriptor.
pub fn set_blocking(f: &File) -> io::Result<()> {
    // SAFETY: plain fcntl calls on an owned descriptor.
    unsafe {
        let fl = libc::fcntl(f.as_raw_fd(), libc::F_GETFL);
        if fl < 0 || libc::fcntl(f.as_raw_fd(), libc::F_SETFL, fl & !libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Waits until `fd` is readable.  `timeout_ms < 0` waits forever.  Returns
/// false on timeout.
pub fn poll_readable(fds: &[RawFd], timeout_ms: i32) -> io::Result<Vec<bool>> {
    let mut p: Vec<libc::pollfd> =
        fds.iter().map(|fd| libc::pollfd { fd: *fd, events: libc::POLLIN, revents: 0 }).collect();
    loop {
        // SAFETY: `p` is a valid array of `p.len()` pollfds.
        let r = unsafe { libc::poll(p.as_mut_ptr(), p.len() as libc::nfds_t, timeout_ms) };
        if r >= 0 {
            return Ok(p.iter().map(|e| e.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0).collect());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

pub fn read_trimmed(path: &Path) -> io::Result<String> {
    Ok(std::fs::read_to_string(path)?.trim().to_string())
}

pub fn read_u32(path: &Path) -> io::Result<u32> {
    read_trimmed(path)?
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("{}: not a number", path.display())))
}
