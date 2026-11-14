//! DSCP and 802.1p marking for SIP and RTP sockets.
//!
//! The voice VLAN device maps socket priority 5 to the RTP 802.1p value and
//! priority 3 to the SIP value, as the stock firmware does, so a socket
//! needs both its DSCP and its priority set.

use std::io;
use std::os::fd::AsRawFd;

use crate::sys;

/// Socket priority the VLAN egress map translates to the RTP 802.1p value.
pub const SKB_PRIORITY_RTP: u32 = 5;
/// Socket priority the VLAN egress map translates to the SIP 802.1p value.
pub const SKB_PRIORITY_SIP: u32 = 3;

/// What kind of traffic a socket carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrafficClass {
    Sip,
    Rtp,
}

/// The marking values in force, after LLDP-MED overrides and defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QosMarks {
    pub rtp_dscp: u8,
    pub sip_dscp: u8,
    /// 802.1p values; only meaningful on a tagged voice VLAN.
    pub rtp_pcp: u8,
    pub sip_pcp: u8,
}

impl Default for QosMarks {
    /// The stock fallbacks: RTP priority 6 and DSCP 46, SIP priority 3 and DSCP 26.
    fn default() -> Self {
        Self {
            rtp_dscp: 46,
            sip_dscp: 26,
            rtp_pcp: 6,
            sip_pcp: 3,
        }
    }
}

impl QosMarks {
    pub fn dscp(&self, class: TrafficClass) -> u8 {
        match class {
            TrafficClass::Sip => self.sip_dscp,
            TrafficClass::Rtp => self.rtp_dscp,
        }
    }
}

/// Set the DSCP on a socket (both the IPv4 TOS byte and, when the socket is
/// IPv6, the traffic class).
pub fn set_dscp(sock: &impl AsRawFd, dscp: u8) -> io::Result<()> {
    let tos = ((dscp & 0x3f) as i32) << 2;
    let fd = sock.as_raw_fd();
    let v4 = sys::setsockopt(fd, libc::IPPROTO_IP, libc::IP_TOS, &tos);
    let v6 = sys::setsockopt(fd, libc::IPPROTO_IPV6, libc::IPV6_TCLASS, &tos);
    if v4.is_err() && v6.is_err() {
        v4
    } else {
        Ok(())
    }
}

/// Set the socket priority.  Note that the kernel resets the priority from
/// the TOS byte whenever `IP_TOS` is set, so call this after [`set_dscp`].
pub fn set_priority(sock: &impl AsRawFd, priority: u32) -> io::Result<()> {
    sys::setsockopt(
        sock.as_raw_fd(),
        libc::SOL_SOCKET,
        libc::SO_PRIORITY,
        &(priority as i32),
    )
}

/// Apply the DSCP and socket priority for a traffic class.
pub fn mark_socket(sock: &impl AsRawFd, class: TrafficClass, marks: &QosMarks) -> io::Result<()> {
    set_dscp(sock, marks.dscp(class))?;
    set_priority(
        sock,
        match class {
            TrafficClass::Sip => SKB_PRIORITY_SIP,
            TrafficClass::Rtp => SKB_PRIORITY_RTP,
        },
    )
}

/// Bind a socket to one interface, so its traffic leaves on the voice
/// interface regardless of the routing table.
pub fn bind_to_device(sock: &impl AsRawFd, ifname: &str) -> io::Result<()> {
    sys::setsockopt_bytes(
        sock.as_raw_fd(),
        libc::SOL_SOCKET,
        libc::SO_BINDTODEVICE,
        ifname.as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_a_udp_socket() {
        let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        mark_socket(&s, TrafficClass::Rtp, &QosMarks::default()).unwrap();
        let mut tos: i32 = 0;
        let mut len = std::mem::size_of::<i32>() as libc::socklen_t;
        // SAFETY: `tos` and `len` are valid for the call.
        unsafe {
            libc::getsockopt(
                s.as_raw_fd(),
                libc::IPPROTO_IP,
                libc::IP_TOS,
                &mut tos as *mut _ as *mut _,
                &mut len,
            )
        };
        assert_eq!(tos, 46 << 2);
        let mut prio: i32 = 0;
        // SAFETY: `prio` and `len` are valid for the call.
        unsafe {
            libc::getsockopt(
                s.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PRIORITY,
                &mut prio as *mut _ as *mut _,
                &mut len,
            )
        };
        assert_eq!(prio, 5);
    }
}
