// SPDX-License-Identifier: GPL-2.0-or-later
//! Sockets for media: RTP/RTCP port pairs and multicast paging.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

/// An RTP socket on an even port with its RTCP socket on the next port.
pub struct RtpSockets {
    pub rtp: UdpSocket,
    pub rtcp: UdpSocket,
}

impl RtpSockets {
    pub fn port(&self) -> u16 {
        self.rtp.local_addr().map(|a| a.port()).unwrap_or(0)
    }
}

/// Binds an RTP/RTCP pair on `ip` within `ports` (inclusive), starting the
/// search at `start` (so that successive calls use different ports).
pub fn bind_rtp_pair(ip: IpAddr, ports: (u16, u16), start: u16) -> io::Result<RtpSockets> {
    let (lo, hi) = (ports.0 & !1, ports.1);
    if hi <= lo {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty RTP port range"));
    }
    let count = u32::from(hi - lo) / 2 + 1;
    let first = (u32::from(start.clamp(lo, hi) - lo) / 2) % count;
    for i in 0..count {
        let port = lo + (((first + i) % count) * 2) as u16;
        if port == 0 || port >= hi {
            continue;
        }
        let Ok(rtp) = UdpSocket::bind(SocketAddr::new(ip, port)) else { continue };
        let Ok(rtcp) = UdpSocket::bind(SocketAddr::new(ip, port + 1)) else { continue };
        return Ok(RtpSockets { rtp, rtcp });
    }
    Err(io::Error::new(io::ErrorKind::AddrInUse, "no free RTP port pair"))
}

/// A socket receiving a multicast page: bound to the group's port and
/// joined to the group on `iface` (the phone's address, or unspecified).
pub fn bind_multicast_rx(group: Ipv4Addr, port: u16, iface: Ipv4Addr) -> io::Result<UdpSocket> {
    if !group.is_multicast() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a multicast address"));
    }
    let sock = bind_reuse(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port))?;
    sock.join_multicast_v4(&group, &iface)?;
    Ok(sock)
}

/// A socket for sending a multicast page from `iface`.
pub fn bind_multicast_tx(iface: Ipv4Addr, ttl: u32) -> io::Result<UdpSocket> {
    let sock = UdpSocket::bind(SocketAddr::new(IpAddr::V4(iface), 0))?;
    sock.set_multicast_ttl_v4(ttl)?;
    sock.set_multicast_loop_v4(false)?;
    Ok(sock)
}

/// `bind` with `SO_REUSEADDR`, so several listeners can share a page port.
fn bind_reuse(addr: SocketAddr) -> io::Result<UdpSocket> {
    use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
    let SocketAddr::V4(v4) = addr else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "IPv4 only"));
    };
    // SAFETY: plain socket calls on a descriptor owned here; the address
    // structure is initialized and its length passed.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let owned = OwnedFd::from_raw_fd(fd);
        let one: libc::c_int = 1;
        if libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, (&raw const one).cast(), size_of::<libc::c_int>() as libc::socklen_t) < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut sa: libc::sockaddr_in = std::mem::zeroed();
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_port = v4.port().to_be();
        sa.sin_addr = libc::in_addr { s_addr: u32::from_ne_bytes(v4.ip().octets()) };
        if libc::bind(fd, (&raw const sa).cast(), size_of::<libc::sockaddr_in>() as libc::socklen_t) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(UdpSocket::from_raw_fd(owned.into_raw_fd()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtp_pairs_are_even_and_adjacent() {
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let a = bind_rtp_pair(ip, (30_000, 30_100), 30_000).unwrap();
        let b = bind_rtp_pair(ip, (30_000, 30_100), 30_000).unwrap();
        assert_eq!(a.port() % 2, 0);
        assert_eq!(a.rtcp.local_addr().unwrap().port(), a.port() + 1);
        assert_ne!(a.port(), b.port());
        assert!(bind_rtp_pair(ip, (5, 5), 5).is_err());
        assert!(bind_multicast_rx(Ipv4Addr::new(10, 0, 0, 1), 5000, Ipv4Addr::UNSPECIFIED).is_err());
    }
}
