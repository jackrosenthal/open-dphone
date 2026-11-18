//! Pseudo-terminals for interactive SSH sessions.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use tokio::io::unix::AsyncFd;

use crate::sys;

pub struct Pty {
    pub master: OwnedFd,
    pub slave: OwnedFd,
}

/// Open a master/slave pair with the given window size.
pub fn open(cols: u16, rows: u16) -> io::Result<Pty> {
    // SAFETY: plain open(2) on a constant path; the result is checked.
    let fd = unsafe { libc::open(c"/dev/ptmx".as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a fresh descriptor we own.
    let master = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut unlock: libc::c_int = 0;
    sys::ioctl_int(&master, libc::TIOCSPTLCK as libc::c_ulong, &mut unlock)?;
    let mut number: libc::c_int = 0;
    sys::ioctl_int(&master, libc::TIOCGPTN as libc::c_ulong, &mut number)?;
    let path = sys::cstring(format!("/dev/pts/{number}"))?;
    // SAFETY: path is a valid C string.
    let sfd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if sfd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: sfd is a fresh descriptor we own.
    let slave = unsafe { OwnedFd::from_raw_fd(sfd) };
    set_size(master.as_raw_fd(), cols, rows)?;
    Ok(Pty { master, slave })
}

pub fn set_size(fd: RawFd, cols: u16, rows: u16) -> io::Result<()> {
    let ws = libc::winsize { ws_row: rows.max(1), ws_col: cols.max(1), ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: ws is a valid winsize for TIOCSWINSZ.
    if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ as _, &ws) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The master side, readable and writable from async code.
pub struct AsyncMaster {
    fd: AsyncFd<OwnedFd>,
}

impl AsyncMaster {
    pub fn new(master: OwnedFd) -> io::Result<Self> {
        sys::set_nonblocking(master.as_raw_fd(), true)?;
        // SAFETY: the descriptor is owned by the AsyncFd for its whole life
        // and is closed only when it is dropped.
        Ok(AsyncMaster { fd: unsafe { AsyncFd::register(master)? } })
    }

    pub fn raw(&self) -> RawFd {
        self.fd.get_ref().as_raw_fd()
    }

    /// Read some bytes.  `Ok(0)` or `EIO` means every slave is closed.
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut guard = self.fd.readable().await?;
            let result = guard.try_io(|fd| {
                // SAFETY: buf is writable for its length.
                let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
            });
            match result {
                Ok(r) => return r,
                Err(_would_block) => continue,
            }
        }
    }

    pub async fn write_all(&self, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            let mut guard = self.fd.writable().await?;
            let result = guard.try_io(|fd| {
                // SAFETY: data is readable for its length.
                let n = unsafe { libc::write(fd.as_raw_fd(), data.as_ptr() as *const libc::c_void, data.len()) };
                if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
            });
            match result {
                Ok(Ok(n)) => data = &data[n..],
                Ok(Err(e)) => return Err(e),
                Err(_would_block) => continue,
            }
        }
        Ok(())
    }
}
