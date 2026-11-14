//! `AF_PACKET` sockets for the protocols that run below IP: DHCP before an
//! address exists, ARP probes, and LLDP.

use std::io;
use std::mem;
use std::os::fd::AsRawFd;

use crate::sys::{self, Fd, MacAddr};

pub const ETH_P_IP: u16 = 0x0800;
pub const ETH_P_ARP: u16 = 0x0806;
pub const ETH_P_LLDP: u16 = 0x88cc;

const PACKET_OUTGOING: u8 = 4;
const PACKET_ADD_MEMBERSHIP: i32 = 1;
const PACKET_MR_MULTICAST: u16 = 0;

/// A datagram-mode packet socket bound to one interface and one EtherType.
/// The kernel adds and strips the Ethernet header.
pub struct PacketSocket {
    fd: Fd,
    ifindex: u32,
    ethertype: u16,
}

fn sockaddr_ll(ifindex: u32, ethertype: u16, dst: Option<MacAddr>) -> libc::sockaddr_ll {
    // SAFETY: zero is a valid bit pattern for `sockaddr_ll`.
    let mut sa: libc::sockaddr_ll = unsafe { mem::zeroed() };
    sa.sll_family = libc::AF_PACKET as u16;
    sa.sll_protocol = ethertype.to_be();
    sa.sll_ifindex = ifindex as i32;
    if let Some(d) = dst {
        sa.sll_halen = 6;
        sa.sll_addr[..6].copy_from_slice(&d.0);
    }
    sa
}

impl PacketSocket {
    pub fn new(ifindex: u32, ethertype: u16) -> io::Result<Self> {
        let fd = sys::socket(libc::AF_PACKET, libc::SOCK_DGRAM, ethertype.to_be() as i32)?;
        let sa = sockaddr_ll(ifindex, ethertype, None);
        // SAFETY: `sa` is a valid `sockaddr_ll` of the length passed.
        sys::cvt(unsafe {
            libc::bind(
                fd.as_raw_fd(),
                &sa as *const _ as *const libc::sockaddr,
                mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        })?;
        Ok(Self {
            fd: Fd::new(fd)?,
            ifindex,
            ethertype,
        })
    }

    /// Ask the interface to receive frames sent to a multicast address.
    pub fn add_multicast(&self, mac: MacAddr) -> io::Result<()> {
        #[repr(C)]
        struct PacketMreq {
            ifindex: i32,
            ty: u16,
            alen: u16,
            address: [u8; 8],
        }
        let mut mreq = PacketMreq {
            ifindex: self.ifindex as i32,
            ty: PACKET_MR_MULTICAST,
            alen: 6,
            address: [0; 8],
        };
        mreq.address[..6].copy_from_slice(&mac.0);
        sys::setsockopt(
            self.fd.raw(),
            libc::SOL_PACKET,
            PACKET_ADD_MEMBERSHIP,
            &mreq,
        )
    }

    /// Set the socket priority (and so the 802.1p value on a VLAN device).
    pub fn set_priority(&self, prio: u32) -> io::Result<()> {
        sys::setsockopt(
            self.fd.raw(),
            libc::SOL_SOCKET,
            libc::SO_PRIORITY,
            &(prio as i32),
        )
    }

    /// Receive one frame's payload and its source address.  Frames this
    /// host sent are skipped.
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<(usize, MacAddr)> {
        loop {
            let (n, sa) = self
                .fd
                .read_with(|fd| {
                    // SAFETY: zero is a valid bit pattern for `sockaddr_ll`.
                    let mut sa: libc::sockaddr_ll = unsafe { mem::zeroed() };
                    let mut len = mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;
                    // SAFETY: the buffer and address pointers are valid for the lengths passed.
                    let n = sys::cvt_size(unsafe {
                        libc::recvfrom(
                            fd,
                            buf.as_mut_ptr() as *mut libc::c_void,
                            buf.len(),
                            0,
                            &mut sa as *mut _ as *mut libc::sockaddr,
                            &mut len,
                        )
                    })?;
                    Ok((n, sa))
                })
                .await?;
            if sa.sll_pkttype == PACKET_OUTGOING {
                continue;
            }
            let mut mac = [0u8; 6];
            mac.copy_from_slice(&sa.sll_addr[..6]);
            return Ok((n, MacAddr(mac)));
        }
    }

    pub async fn send(&self, dst: MacAddr, payload: &[u8]) -> io::Result<usize> {
        let sa = sockaddr_ll(self.ifindex, self.ethertype, Some(dst));
        self.fd
            .write_with(|fd| {
                // SAFETY: the buffer and address pointers are valid for the lengths passed.
                sys::cvt_size(unsafe {
                    libc::sendto(
                        fd,
                        payload.as_ptr() as *const libc::c_void,
                        payload.len(),
                        libc::MSG_NOSIGNAL,
                        &sa as *const _ as *const libc::sockaddr,
                        mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
                    )
                })
            })
            .await
    }
}

/// The Internet checksum over `data`, starting from `initial`.
pub fn checksum(data: &[u8], initial: u32) -> u16 {
    let mut sum = initial;
    let mut chunks = data.chunks_exact(2);
    for c in &mut chunks {
        sum += u16::from_be_bytes([c[0], c[1]]) as u32;
    }
    if let [last] = chunks.remainder() {
        sum += (*last as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Wrap a UDP payload in IPv4 and UDP headers.
pub fn build_udp4(
    src: std::net::Ipv4Addr,
    sport: u16,
    dst: std::net::Ipv4Addr,
    dport: u16,
    tos: u8,
    payload: &[u8],
) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let total = 20 + udp_len;
    let mut p = Vec::with_capacity(total);
    p.extend_from_slice(&[0x45, tos]);
    p.extend_from_slice(&(total as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0, 0x40, 0, 64, 17, 0, 0]);
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    let ip_sum = checksum(&p[..20], 0);
    p[10..12].copy_from_slice(&ip_sum.to_be_bytes());
    p.extend_from_slice(&sport.to_be_bytes());
    p.extend_from_slice(&dport.to_be_bytes());
    p.extend_from_slice(&(udp_len as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(payload);
    let mut pseudo = 0u32;
    for c in src.octets().chunks(2).chain(dst.octets().chunks(2)) {
        pseudo += u16::from_be_bytes([c[0], c[1]]) as u32;
    }
    pseudo += 17 + udp_len as u32;
    let mut udp_sum = checksum(&p[20..], pseudo);
    if udp_sum == 0 {
        udp_sum = 0xffff;
    }
    p[26..28].copy_from_slice(&udp_sum.to_be_bytes());
    p
}

/// A parsed IPv4 UDP datagram.
pub struct Udp4<'a> {
    #[allow(dead_code)]
    pub src: std::net::Ipv4Addr,
    #[allow(dead_code)]
    pub dst: std::net::Ipv4Addr,
    pub sport: u16,
    pub dport: u16,
    pub payload: &'a [u8],
}

/// Parse an IPv4 packet carrying UDP.  The IP header checksum is verified;
/// the UDP checksum is not, because locally generated packets seen through a
/// packet socket can carry a partial (offloaded) checksum.
pub fn parse_udp4(p: &[u8]) -> Option<Udp4<'_>> {
    if p.len() < 28 || p[0] >> 4 != 4 {
        return None;
    }
    let ihl = (p[0] & 0xf) as usize * 4;
    if ihl < 20 || p.len() < ihl + 8 || p[9] != 17 {
        return None;
    }
    if checksum(&p[..ihl], 0) != 0 {
        return None;
    }
    // Fragments are not reassembled.
    if u16::from_be_bytes([p[6], p[7]]) & 0x3fff != 0 {
        return None;
    }
    let total = u16::from_be_bytes([p[2], p[3]]) as usize;
    if total < ihl + 8 || total > p.len() {
        return None;
    }
    let udp = &p[ihl..total];
    let ulen = u16::from_be_bytes([udp[4], udp[5]]) as usize;
    if ulen < 8 || ulen > udp.len() {
        return None;
    }
    Some(Udp4 {
        src: std::net::Ipv4Addr::new(p[12], p[13], p[14], p[15]),
        dst: std::net::Ipv4Addr::new(p[16], p[17], p[18], p[19]),
        sport: u16::from_be_bytes([udp[0], udp[1]]),
        dport: u16::from_be_bytes([udp[2], udp[3]]),
        payload: &udp[8..ulen],
    })
}

/// A raw packet socket bound to one interface: whole Ethernet frames in
/// both directions, limited to one EtherType by a kernel filter.  Unlike
/// [`PacketSocket`], it sees frames that a bridge above the interface
/// consumes, and it can send with any source address.
pub struct RawSocket {
    fd: Fd,
    ethertype: u16,
}

impl RawSocket {
    pub fn new(ifindex: u32, ethertype: u16) -> io::Result<Self> {
        const ETH_P_ALL: u16 = 0x0003;
        let fd = sys::socket(libc::AF_PACKET, libc::SOCK_RAW, ETH_P_ALL.to_be() as i32)?;
        // Accept only frames whose EtherType field matches.
        let program = [
            libc::sock_filter { code: 0x28, jt: 0, jf: 0, k: 12 }, // load the half-word at offset 12
            libc::sock_filter { code: 0x15, jt: 0, jf: 1, k: ethertype as u32 }, // equal?
            libc::sock_filter { code: 0x06, jt: 0, jf: 0, k: 0xffff }, // accept
            libc::sock_filter { code: 0x06, jt: 0, jf: 0, k: 0 }, // drop
        ];
        let fprog = libc::sock_fprog { len: program.len() as u16, filter: program.as_ptr() as *mut libc::sock_filter };
        sys::setsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_ATTACH_FILTER, &fprog)?;
        let sa = sockaddr_ll(ifindex, ETH_P_ALL, None);
        // SAFETY: `sa` is a valid `sockaddr_ll` of the length passed.
        sys::cvt(unsafe {
            libc::bind(fd.as_raw_fd(), &sa as *const _ as *const libc::sockaddr, mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t)
        })?;
        Ok(Self { fd: Fd::new(fd)?, ethertype })
    }

    /// Receive one frame, Ethernet header included.  Frames this host sent,
    /// and frames that slipped in before the filter was attached, are skipped.
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let (n, pkttype) = self
                .fd
                .read_with(|fd| {
                    // SAFETY: zero is a valid bit pattern for `sockaddr_ll`.
                    let mut sa: libc::sockaddr_ll = unsafe { mem::zeroed() };
                    let mut len = mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;
                    // SAFETY: the buffer and address pointers are valid for the lengths passed.
                    let n = sys::cvt_size(unsafe {
                        libc::recvfrom(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0, &mut sa as *mut _ as *mut libc::sockaddr, &mut len)
                    })?;
                    Ok((n, sa.sll_pkttype))
                })
                .await?;
            if pkttype == PACKET_OUTGOING || n < 14 || buf[12..14] != self.ethertype.to_be_bytes() {
                continue;
            }
            return Ok(n);
        }
    }

    /// Send one complete Ethernet frame.
    pub async fn send(&self, frame: &[u8]) -> io::Result<usize> {
        self.fd.send(frame).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn rfc1071_checksum() {
        // Example from RFC 1071 section 3.
        let data = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        assert_eq!(checksum(&data, 0), !0xddf2);
    }

    #[test]
    fn udp_round_trip() {
        let p = build_udp4(
            Ipv4Addr::UNSPECIFIED,
            68,
            Ipv4Addr::BROADCAST,
            67,
            0,
            b"hello",
        );
        let u = parse_udp4(&p).unwrap();
        assert_eq!((u.sport, u.dport, u.payload), (68, 67, &b"hello"[..]));
        assert_eq!(u.src, Ipv4Addr::UNSPECIFIED);
        // The UDP checksum verifies against the pseudo-header.
        let mut pseudo = 0u32;
        for c in p[12..20].chunks(2) {
            pseudo += u16::from_be_bytes([c[0], c[1]]) as u32;
        }
        pseudo += 17 + (p.len() - 20) as u32;
        assert_eq!(checksum(&p[20..], pseudo), 0);
        let mut bad = p.clone();
        bad[8] ^= 1;
        assert!(parse_udp4(&bad).is_none());
    }
}
