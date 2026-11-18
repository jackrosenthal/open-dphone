//! Thin wrappers over the system calls this crate needs.  Everything here is
//! non-blocking and meant to be driven through `tokio::io::unix::AsyncFd`.

use std::io;
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

pub(crate) fn cvt(ret: libc::c_int) -> io::Result<libc::c_int> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub(crate) fn cvt_size(ret: libc::ssize_t) -> io::Result<usize> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret as usize)
    }
}

pub(crate) fn socket(domain: i32, ty: i32, proto: i32) -> io::Result<OwnedFd> {
    // SAFETY: plain socket(2); the result is checked before being wrapped.
    let fd =
        cvt(unsafe { libc::socket(domain, ty | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, proto) })?;
    // SAFETY: `fd` is a freshly created descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

pub(crate) fn setsockopt<T>(fd: RawFd, level: i32, name: i32, value: &T) -> io::Result<()> {
    // SAFETY: `value` is a valid `T` for the length passed.
    cvt(unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            value as *const T as *const libc::c_void,
            mem::size_of::<T>() as libc::socklen_t,
        )
    })?;
    Ok(())
}

pub(crate) fn setsockopt_bytes(fd: RawFd, level: i32, name: i32, value: &[u8]) -> io::Result<()> {
    // SAFETY: the pointer and length describe `value`.
    cvt(unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            value.as_ptr() as *const libc::c_void,
            value.len() as libc::socklen_t,
        )
    })?;
    Ok(())
}

/// Interface index for a name, or `None` when there is no such interface.
pub fn if_index(name: &str) -> Option<u32> {
    let c = std::ffi::CString::new(name).ok()?;
    // SAFETY: `c` is a valid NUL-terminated string.
    let idx = unsafe { libc::if_nametoindex(c.as_ptr()) };
    if idx == 0 { None } else { Some(idx) }
}

/// A non-blocking descriptor registered with the tokio reactor.
pub(crate) struct Fd(AsyncFd<OwnedFd>);

impl Fd {
    pub(crate) fn new(fd: OwnedFd) -> io::Result<Self> {
        // SAFETY: `fd` is an open descriptor owned by the returned value,
        // which neither closes nor replaces it while registered.
        Ok(Self(unsafe {
            AsyncFd::register_with_interest(fd, Interest::READABLE | Interest::WRITABLE)?
        }))
    }

    pub(crate) fn raw(&self) -> RawFd {
        self.0.get_ref().as_raw_fd()
    }

    pub(crate) async fn read_with<R>(
        &self,
        mut f: impl FnMut(RawFd) -> io::Result<R>,
    ) -> io::Result<R> {
        loop {
            let mut guard = self.0.readable().await?;
            match guard.try_io(|fd| f(fd.get_ref().as_raw_fd())) {
                Ok(r) => return r,
                Err(_would_block) => continue,
            }
        }
    }

    pub(crate) async fn write_with<R>(
        &self,
        mut f: impl FnMut(RawFd) -> io::Result<R>,
    ) -> io::Result<R> {
        loop {
            let mut guard = self.0.writable().await?;
            match guard.try_io(|fd| f(fd.get_ref().as_raw_fd())) {
                Ok(r) => return r,
                Err(_would_block) => continue,
            }
        }
    }

    pub(crate) async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_with(|fd| {
            // SAFETY: the pointer and length describe `buf`.
            cvt_size(unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) })
        })
        .await
    }

    pub(crate) async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.write_with(|fd| {
            // SAFETY: the pointer and length describe `buf`.
            cvt_size(unsafe {
                libc::send(
                    fd,
                    buf.as_ptr() as *const libc::c_void,
                    buf.len(),
                    libc::MSG_NOSIGNAL,
                )
            })
        })
        .await
    }
}

/// Set the wall clock.  Uses the 64-bit time system call directly on 32-bit
/// targets so the result does not depend on the C library's `time_t` width.
pub fn set_realtime_clock(secs: i64, nanos: u32) -> io::Result<()> {
    #[cfg(target_pointer_width = "32")]
    {
        #[repr(C)]
        struct KernelTimespec {
            tv_sec: i64,
            tv_nsec: i64,
        }
        const SYS_CLOCK_SETTIME64: libc::c_long = 404;
        let ts = KernelTimespec {
            tv_sec: secs,
            tv_nsec: nanos as i64,
        };
        // SAFETY: `ts` is a valid `__kernel_timespec` for the duration of the call.
        let r = unsafe {
            libc::syscall(
                SYS_CLOCK_SETTIME64,
                libc::CLOCK_REALTIME,
                &ts as *const KernelTimespec,
            )
        };
        if r < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(target_pointer_width = "32"))]
    {
        // SAFETY: zero is a valid bit pattern for `timespec`.
        let mut ts: libc::timespec = unsafe { mem::zeroed() };
        ts.tv_sec = secs as _;
        ts.tv_nsec = nanos as _;
        // SAFETY: `ts` is a valid `timespec`.
        cvt(unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) })?;
        Ok(())
    }
}

/// A six-byte Ethernet address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    pub const BROADCAST: MacAddr = MacAddr([0xff; 6]);

    /// Parse `aa:bb:cc:dd:ee:ff`, `aa-bb-...`, or twelve bare hex digits.
    pub fn parse(s: &str) -> Option<Self> {
        let hex: Vec<u8> = s
            .trim()
            .bytes()
            .filter(|b| !matches!(b, b':' | b'-' | b'.'))
            .collect();
        if hex.len() != 12 {
            return None;
        }
        let mut out = [0u8; 6];
        for (i, pair) in hex.chunks(2).enumerate() {
            let s = std::str::from_utf8(pair).ok()?;
            out[i] = u8::from_str_radix(s, 16).ok()?;
        }
        Some(MacAddr(out))
    }

    /// Unicast, and not all zeroes.
    pub fn is_valid_unicast(&self) -> bool {
        self.0 != [0; 6] && self.0[0] & 1 == 0
    }

    /// Twelve lowercase hex digits without separators.
    pub fn bare_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl std::fmt::Display for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let m = self.0;
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            m[0], m[1], m[2], m[3], m[4], m[5]
        )
    }
}

impl std::fmt::Debug for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

/// Extract `ENETCFG_MAC` from the text of `/nvdata/etc/enet.cfg`.
pub fn parse_enet_cfg_mac(text: &str) -> Option<MacAddr> {
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        if let Some(rest) = line.strip_prefix("ENETCFG_MAC") {
            let rest = rest.trim_start();
            if let Some(v) = rest.strip_prefix('=') {
                let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                if let Some(mac) = MacAddr::parse(v) {
                    return Some(mac);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_parse_and_format() {
        let m = MacAddr::parse("00:0F:D3:12:34:56").unwrap();
        assert_eq!(m.to_string(), "00:0f:d3:12:34:56");
        assert_eq!(m.bare_hex(), "000fd3123456");
        assert_eq!(MacAddr::parse("000fd3123456"), Some(m));
        assert!(MacAddr::parse("00:0f:d3").is_none());
        assert!(m.is_valid_unicast());
        assert!(!MacAddr::BROADCAST.is_valid_unicast());
    }

    #[test]
    fn enet_cfg() {
        let text = "# comment\nENETCFG_FOO=1\nENETCFG_MAC=00:0f:d3:aa:bb:cc\n";
        assert_eq!(
            parse_enet_cfg_mac(text).unwrap().to_string(),
            "00:0f:d3:aa:bb:cc"
        );
        assert_eq!(
            parse_enet_cfg_mac("ENETCFG_MAC=\"000FD3AABBCC\"\n")
                .unwrap()
                .bare_hex(),
            "000fd3aabbcc"
        );
        assert!(parse_enet_cfg_mac("ENETCFG_MAC=\n").is_none());
    }
}
