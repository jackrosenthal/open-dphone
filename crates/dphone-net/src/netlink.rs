//! A small rtnetlink client: links, addresses, routes, bridges, VLANs, and
//! link monitoring.  Hand-rolled so the firmware carries no netlink
//! dependency tree; only the messages the phone needs are implemented.

use std::io;
use std::mem;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::sys::{self, Fd, MacAddr};

// Message types.
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const RTM_NEWLINK: u16 = 16;
const RTM_DELLINK: u16 = 17;
const RTM_GETLINK: u16 = 18;
const RTM_SETLINK: u16 = 19;
const RTM_NEWADDR: u16 = 20;
const RTM_DELADDR: u16 = 21;
const RTM_GETADDR: u16 = 22;
const RTM_NEWROUTE: u16 = 24;
const RTM_DELROUTE: u16 = 25;
const RTM_GETROUTE: u16 = 26;

// Header flags.
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_MULTI: u16 = 0x2;
const NLM_F_ACK: u16 = 0x4;
const NLM_F_DUMP: u16 = 0x300;
const NLM_F_REPLACE: u16 = 0x100;
const NLM_F_EXCL: u16 = 0x200;
const NLM_F_CREATE: u16 = 0x400;

// Link attributes.
const IFLA_ADDRESS: u16 = 1;
const IFLA_IFNAME: u16 = 3;
const IFLA_MTU: u16 = 4;
const IFLA_LINK: u16 = 5;
const IFLA_MASTER: u16 = 10;
const IFLA_OPERSTATE: u16 = 16;
const IFLA_LINKINFO: u16 = 18;
const IFLA_AF_SPEC: u16 = 26;
const IFLA_CARRIER: u16 = 33;
const IFLA_INFO_KIND: u16 = 1;
const IFLA_INFO_DATA: u16 = 2;

const IFLA_VLAN_ID: u16 = 1;
const IFLA_VLAN_EGRESS_QOS: u16 = 3;
const IFLA_VLAN_INGRESS_QOS: u16 = 4;
const IFLA_VLAN_QOS_MAPPING: u16 = 1;

const IFLA_BR_FORWARD_DELAY: u16 = 1;
const IFLA_BR_STP_STATE: u16 = 5;
const IFLA_BR_VLAN_FILTERING: u16 = 7;
const IFLA_BR_GROUP_FWD_MASK: u16 = 9;
const IFLA_BR_VLAN_DEFAULT_PVID: u16 = 39;

const IFLA_BRIDGE_FLAGS: u16 = 0;
const IFLA_BRIDGE_VLAN_INFO: u16 = 2;
const BRIDGE_FLAGS_MASTER: u16 = 1;
const BRIDGE_FLAGS_SELF: u16 = 2;

const VETH_INFO_PEER: u16 = 1;

// Address attributes.
const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;
const IFA_BROADCAST: u16 = 4;
const IFA_FLAGS: u16 = 8;

// Route attributes.
const RTA_DST: u16 = 1;
const RTA_OIF: u16 = 4;
const RTA_GATEWAY: u16 = 5;
const RTA_PRIORITY: u16 = 6;

const RT_TABLE_MAIN: u8 = 254;
const RT_SCOPE_UNIVERSE: u8 = 0;
const RT_SCOPE_LINK: u8 = 253;
const RT_SCOPE_NOWHERE: u8 = 255;
const RTN_UNICAST: u8 = 1;
/// Route protocol for routes this crate installs from DHCP.
pub const RTPROT_DHCP: u8 = 16;
/// Route protocol for routes this crate installs from static configuration.
pub const RTPROT_STATIC: u8 = 4;
/// Route protocol the kernel uses for routes learned from router advertisements.
pub const RTPROT_RA: u8 = 9;

const NLA_F_NESTED: u16 = 0x8000;
const NLA_TYPE_MASK: u16 = 0x3fff;

const AF_BRIDGE: u8 = 7;

/// `IFF_UP` from `linux/if.h`.
pub const IFF_UP: u32 = 0x1;
/// `IFF_RUNNING`: the kernel's view that the link is operational.
pub const IFF_RUNNING: u32 = 0x40;
/// `IFF_LOWER_UP`: carrier present.
pub const IFF_LOWER_UP: u32 = 0x1_0000;

/// Multicast groups for [`Monitor`].
const RTMGRP_LINK: u32 = 0x1;
const RTMGRP_IPV4_IFADDR: u32 = 0x10;
const RTMGRP_IPV6_IFADDR: u32 = 0x100;
/// `RTNLGRP_ND_USEROPT` (group 20) as a legacy bitmask.
const RTMGRP_ND_USEROPT: u32 = 1 << 19;
const RTM_NEWNDUSEROPT: u16 = 68;
const ND_OPT_RDNSS: u8 = 25;

/// `IF_OPER_*` values from RFC 2863 as the kernel reports them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperState {
    Unknown,
    NotPresent,
    Down,
    LowerLayerDown,
    Testing,
    Dormant,
    Up,
}

impl OperState {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::NotPresent,
            2 => Self::Down,
            3 => Self::LowerLayerDown,
            4 => Self::Testing,
            5 => Self::Dormant,
            6 => Self::Up,
            _ => Self::Unknown,
        }
    }
}

/// One network interface as reported by the kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub index: u32,
    pub name: String,
    pub flags: u32,
    pub mac: Option<MacAddr>,
    pub mtu: u32,
    pub master: Option<u32>,
    pub parent: Option<u32>,
    pub kind: Option<String>,
    pub operstate: OperState,
    pub carrier: Option<bool>,
}

impl Link {
    pub fn is_admin_up(&self) -> bool {
        self.flags & IFF_UP != 0
    }

    /// Carrier present.  Mirrors the stock test, which accepted operstate
    /// `up` or `unknown` on an administratively up interface.
    pub fn has_carrier(&self) -> bool {
        if !self.is_admin_up() {
            return false;
        }
        match self.carrier {
            Some(c) => c,
            None => self.flags & IFF_LOWER_UP != 0,
        }
    }
}

/// An address on an interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Addr {
    pub index: u32,
    pub addr: IpAddr,
    pub prefix_len: u8,
    pub scope: u8,
    pub flags: u32,
}

/// A route in the main table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub dst: Option<IpAddr>,
    pub dst_len: u8,
    pub gateway: Option<IpAddr>,
    pub oif: Option<u32>,
    pub metric: u32,
    pub protocol: u8,
    pub table: u8,
    pub v6: bool,
}

/// Flags for a bridge VLAN table entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VlanFlags {
    pub pvid: bool,
    pub untagged: bool,
}

/// Options applied when creating or reconfiguring a bridge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BridgeOpts {
    pub vlan_filtering: bool,
    /// Bit `n` forwards frames addressed to `01:80:c2:00:00:0n`.
    pub group_fwd_mask: u16,
    /// Default PVID for new ports; zero means "none".
    pub default_pvid: u16,
}

/// A netlink message under construction.
struct Msg {
    buf: Vec<u8>,
}

impl Msg {
    fn new(ty: u16, flags: u16) -> Self {
        let mut buf = Vec::with_capacity(128);
        buf.extend_from_slice(&0u32.to_ne_bytes()); // length, patched in `finish`
        buf.extend_from_slice(&ty.to_ne_bytes());
        buf.extend_from_slice(&(flags | NLM_F_REQUEST).to_ne_bytes());
        buf.extend_from_slice(&0u32.to_ne_bytes()); // sequence
        buf.extend_from_slice(&0u32.to_ne_bytes()); // port id
        Self { buf }
    }

    fn pad(&mut self) {
        while !self.buf.len().is_multiple_of(4) {
            self.buf.push(0);
        }
    }

    fn ifinfo(&mut self, family: u8, index: u32, flags: u32, change: u32) -> &mut Self {
        self.buf.extend_from_slice(&[family, 0]);
        self.buf.extend_from_slice(&0u16.to_ne_bytes());
        self.buf.extend_from_slice(&(index as i32).to_ne_bytes());
        self.buf.extend_from_slice(&flags.to_ne_bytes());
        self.buf.extend_from_slice(&change.to_ne_bytes());
        self
    }

    fn ifaddr(&mut self, family: u8, prefix: u8, scope: u8, index: u32) -> &mut Self {
        self.buf.extend_from_slice(&[family, prefix, 0, scope]);
        self.buf.extend_from_slice(&index.to_ne_bytes());
        self
    }

    #[allow(clippy::too_many_arguments)]
    fn rtmsg(
        &mut self,
        family: u8,
        dst_len: u8,
        table: u8,
        protocol: u8,
        scope: u8,
        ty: u8,
    ) -> &mut Self {
        self.buf
            .extend_from_slice(&[family, dst_len, 0, 0, table, protocol, scope, ty]);
        self.buf.extend_from_slice(&0u32.to_ne_bytes());
        self
    }

    fn attr(&mut self, ty: u16, data: &[u8]) -> &mut Self {
        let len = (4 + data.len()) as u16;
        self.buf.extend_from_slice(&len.to_ne_bytes());
        self.buf.extend_from_slice(&ty.to_ne_bytes());
        self.buf.extend_from_slice(data);
        self.pad();
        self
    }

    fn attr_u8(&mut self, ty: u16, v: u8) -> &mut Self {
        self.attr(ty, &[v])
    }

    fn attr_u16(&mut self, ty: u16, v: u16) -> &mut Self {
        self.attr(ty, &v.to_ne_bytes())
    }

    fn attr_u32(&mut self, ty: u16, v: u32) -> &mut Self {
        self.attr(ty, &v.to_ne_bytes())
    }

    fn attr_str(&mut self, ty: u16, s: &str) -> &mut Self {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        self.attr(ty, &v)
    }

    fn attr_ip(&mut self, ty: u16, ip: IpAddr) -> &mut Self {
        match ip {
            IpAddr::V4(a) => self.attr(ty, &a.octets()),
            IpAddr::V6(a) => self.attr(ty, &a.octets()),
        }
    }

    fn nest_start(&mut self, ty: u16) -> usize {
        let pos = self.buf.len();
        self.buf.extend_from_slice(&0u16.to_ne_bytes());
        self.buf
            .extend_from_slice(&(ty | NLA_F_NESTED).to_ne_bytes());
        pos
    }

    fn nest_end(&mut self, pos: usize) {
        let len = (self.buf.len() - pos) as u16;
        self.buf[pos..pos + 2].copy_from_slice(&len.to_ne_bytes());
        self.pad();
    }

    fn finish(mut self, seq: u32) -> Vec<u8> {
        let len = self.buf.len() as u32;
        self.buf[0..4].copy_from_slice(&len.to_ne_bytes());
        self.buf[8..12].copy_from_slice(&seq.to_ne_bytes());
        self.buf
    }
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_ne_bytes([b[off], b[off + 1]])
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_ne_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Iterate over the attributes in `b`.
fn attrs(mut b: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    std::iter::from_fn(move || {
        if b.len() < 4 {
            return None;
        }
        let len = u16_at(b, 0) as usize;
        let ty = u16_at(b, 2) & NLA_TYPE_MASK;
        if len < 4 || len > b.len() {
            return None;
        }
        let data = &b[4..len];
        let next = (len + 3) & !3;
        b = if next >= b.len() { &[] } else { &b[next..] };
        Some((ty, data))
    })
}

/// Split a receive buffer into `(type, flags, seq, payload)` messages.
fn messages(mut b: &[u8]) -> impl Iterator<Item = (u16, u16, u32, &[u8])> {
    std::iter::from_fn(move || {
        if b.len() < 16 {
            return None;
        }
        let len = u32_at(b, 0) as usize;
        if len < 16 || len > b.len() {
            return None;
        }
        let ty = u16_at(b, 4);
        let flags = u16_at(b, 6);
        let seq = u32_at(b, 8);
        let payload = &b[16..len];
        let next = (len + 3) & !3;
        b = if next >= b.len() { &[] } else { &b[next..] };
        Some((ty, flags, seq, payload))
    })
}

fn parse_ip(data: &[u8]) -> Option<IpAddr> {
    match data.len() {
        4 => Some(IpAddr::V4(Ipv4Addr::new(
            data[0], data[1], data[2], data[3],
        ))),
        16 => {
            let mut o = [0u8; 16];
            o.copy_from_slice(data);
            Some(IpAddr::V6(Ipv6Addr::from(o)))
        }
        _ => None,
    }
}

fn cstr(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).into_owned()
}

fn parse_link(p: &[u8]) -> Option<Link> {
    if p.len() < 16 {
        return None;
    }
    let mut link = Link {
        index: u32_at(p, 4),
        name: String::new(),
        flags: u32_at(p, 8),
        mac: None,
        mtu: 0,
        master: None,
        parent: None,
        kind: None,
        operstate: OperState::Unknown,
        carrier: None,
    };
    for (ty, data) in attrs(&p[16..]) {
        match ty {
            IFLA_IFNAME => link.name = cstr(data),
            IFLA_ADDRESS if data.len() == 6 => {
                let mut m = [0u8; 6];
                m.copy_from_slice(data);
                link.mac = Some(MacAddr(m));
            }
            IFLA_MTU if data.len() >= 4 => link.mtu = u32_at(data, 0),
            IFLA_MASTER if data.len() >= 4 => link.master = Some(u32_at(data, 0)),
            IFLA_LINK if data.len() >= 4 => link.parent = Some(u32_at(data, 0)),
            IFLA_OPERSTATE if !data.is_empty() => link.operstate = OperState::from_u8(data[0]),
            IFLA_CARRIER if !data.is_empty() => link.carrier = Some(data[0] != 0),
            IFLA_LINKINFO => {
                for (t, d) in attrs(data) {
                    if t == IFLA_INFO_KIND {
                        link.kind = Some(cstr(d));
                    }
                }
            }
            _ => {}
        }
    }
    Some(link)
}

fn parse_addr(p: &[u8]) -> Option<Addr> {
    if p.len() < 8 {
        return None;
    }
    let prefix_len = p[1];
    let mut flags = p[2] as u32;
    let scope = p[3];
    let index = u32_at(p, 4);
    let mut local = None;
    let mut address = None;
    for (ty, data) in attrs(&p[8..]) {
        match ty {
            IFA_LOCAL => local = parse_ip(data),
            IFA_ADDRESS => address = parse_ip(data),
            IFA_FLAGS if data.len() >= 4 => flags = u32_at(data, 0),
            _ => {}
        }
    }
    Some(Addr {
        index,
        addr: local.or(address)?,
        prefix_len,
        scope,
        flags,
    })
}

fn parse_route(p: &[u8]) -> Option<Route> {
    if p.len() < 12 {
        return None;
    }
    let mut r = Route {
        dst: None,
        dst_len: p[1],
        gateway: None,
        oif: None,
        metric: 0,
        protocol: p[5],
        table: p[4],
        v6: p[0] == libc::AF_INET6 as u8,
    };
    for (ty, data) in attrs(&p[12..]) {
        match ty {
            RTA_DST => r.dst = parse_ip(data),
            RTA_GATEWAY => r.gateway = parse_ip(data),
            RTA_OIF if data.len() >= 4 => r.oif = Some(u32_at(data, 0)),
            RTA_PRIORITY if data.len() >= 4 => r.metric = u32_at(data, 0),
            _ => {}
        }
    }
    Some(r)
}

/// Parse an `RTM_NEWNDUSEROPT` payload carrying an RDNSS option.
fn parse_rdnss(p: &[u8]) -> Option<Event> {
    // struct nduseroptmsg is 16 bytes; the ND options follow.
    if p.len() < 16 || p[0] != libc::AF_INET6 as u8 {
        return None;
    }
    let opts_len = u16_at(p, 2) as usize;
    let index = u32_at(p, 4);
    let mut o = p.get(16..16 + opts_len)?;
    while o.len() >= 8 {
        let len = o[1] as usize * 8;
        if len == 0 || len > o.len() {
            break;
        }
        if o[0] == ND_OPT_RDNSS && len >= 24 {
            let lifetime_secs = u32::from_be_bytes([o[4], o[5], o[6], o[7]]);
            let servers = o[8..len]
                .chunks_exact(16)
                .filter_map(parse_ip)
                .filter_map(|ip| {
                    if let IpAddr::V6(a) = ip {
                        Some(a)
                    } else {
                        None
                    }
                })
                .collect();
            return Some(Event::Rdnss {
                index,
                lifetime_secs,
                servers,
            });
        }
        o = &o[len..];
    }
    None
}

fn netlink_socket(groups: u32) -> io::Result<Fd> {
    let fd = sys::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE)?;
    // SAFETY: zero is a valid bit pattern for `sockaddr_nl`.
    let mut sa: libc::sockaddr_nl = unsafe { mem::zeroed() };
    sa.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    sa.nl_groups = groups;
    use std::os::fd::AsRawFd;
    // SAFETY: `sa` is a valid `sockaddr_nl` of the length passed.
    sys::cvt(unsafe {
        libc::bind(
            fd.as_raw_fd(),
            &sa as *const _ as *const libc::sockaddr,
            mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    })?;
    let _ = sys::setsockopt(
        fd.as_raw_fd(),
        libc::SOL_SOCKET,
        libc::SO_RCVBUF,
        &(256 * 1024i32),
    );
    Fd::new(fd)
}

/// A request/response rtnetlink connection.
pub struct Netlink {
    fd: Fd,
    seq: u32,
}

impl Netlink {
    /// Open a connection.  Must be called inside a tokio runtime.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            fd: netlink_socket(0)?,
            seq: 1,
        })
    }

    /// Send `msg` and collect every reply payload of type `want` until the
    /// kernel acknowledges or ends the dump.
    async fn request(&mut self, msg: Msg, want: u16) -> io::Result<Vec<Vec<u8>>> {
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        let is_dump = u16_at(&msg.buf, 6) & NLM_F_DUMP == NLM_F_DUMP;
        let bytes = msg.finish(seq);
        self.fd.send(&bytes).await?;
        let mut out = Vec::new();
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            let n = self.fd.recv(&mut buf).await?;
            for (ty, flags, mseq, payload) in messages(&buf[..n]) {
                if mseq != seq {
                    continue;
                }
                match ty {
                    NLMSG_ERROR => {
                        if payload.len() < 4 {
                            return Err(io::Error::other("short netlink error"));
                        }
                        let code =
                            i32::from_ne_bytes([payload[0], payload[1], payload[2], payload[3]]);
                        if code == 0 {
                            return Ok(out);
                        }
                        return Err(io::Error::from_raw_os_error(-code));
                    }
                    NLMSG_DONE => return Ok(out),
                    t if t == want => {
                        out.push(payload.to_vec());
                        if !is_dump
                            && flags & NLM_F_MULTI == 0
                            && u16_at(&bytes, 6) & NLM_F_ACK == 0
                        {
                            return Ok(out);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    async fn ack(&mut self, mut msg: Msg) -> io::Result<()> {
        let flags = u16_at(&msg.buf, 6) | NLM_F_ACK;
        msg.buf[6..8].copy_from_slice(&flags.to_ne_bytes());
        self.request(msg, 0).await.map(|_| ())
    }

    /// Every interface.
    pub async fn links(&mut self) -> io::Result<Vec<Link>> {
        let mut m = Msg::new(RTM_GETLINK, NLM_F_DUMP);
        m.ifinfo(0, 0, 0, 0);
        let replies = self.request(m, RTM_NEWLINK).await?;
        Ok(replies.iter().filter_map(|p| parse_link(p)).collect())
    }

    /// One interface by name; `Ok(None)` when it does not exist.
    pub async fn link_by_name(&mut self, name: &str) -> io::Result<Option<Link>> {
        let mut m = Msg::new(RTM_GETLINK, 0);
        m.ifinfo(0, 0, 0, 0).attr_str(IFLA_IFNAME, name);
        match self.request(m, RTM_NEWLINK).await {
            Ok(r) => Ok(r.first().and_then(|p| parse_link(p))),
            Err(e) if e.raw_os_error() == Some(libc::ENODEV) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// One interface by index; `Ok(None)` when it does not exist.
    pub async fn link_by_index(&mut self, index: u32) -> io::Result<Option<Link>> {
        let mut m = Msg::new(RTM_GETLINK, 0);
        m.ifinfo(0, index, 0, 0);
        match self.request(m, RTM_NEWLINK).await {
            Ok(r) => Ok(r.first().and_then(|p| parse_link(p))),
            Err(e) if e.raw_os_error() == Some(libc::ENODEV) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Bring an interface administratively up or down.
    pub async fn set_up(&mut self, index: u32, up: bool) -> io::Result<()> {
        let mut m = Msg::new(RTM_NEWLINK, 0);
        m.ifinfo(0, index, if up { IFF_UP } else { 0 }, IFF_UP);
        self.ack(m).await
    }

    /// Set the hardware address.  Most drivers require the link to be down.
    pub async fn set_mac(&mut self, index: u32, mac: MacAddr) -> io::Result<()> {
        let mut m = Msg::new(RTM_NEWLINK, 0);
        m.ifinfo(0, index, 0, 0).attr(IFLA_ADDRESS, &mac.0);
        self.ack(m).await
    }

    pub async fn set_mtu(&mut self, index: u32, mtu: u32) -> io::Result<()> {
        let mut m = Msg::new(RTM_NEWLINK, 0);
        m.ifinfo(0, index, 0, 0).attr_u32(IFLA_MTU, mtu);
        self.ack(m).await
    }

    /// Enslave `index` to `master`, or release it with `None`.
    pub async fn set_master(&mut self, index: u32, master: Option<u32>) -> io::Result<()> {
        let mut m = Msg::new(RTM_NEWLINK, 0);
        m.ifinfo(0, index, 0, 0)
            .attr_u32(IFLA_MASTER, master.unwrap_or(0));
        self.ack(m).await
    }

    fn bridge_info(m: &mut Msg, opts: &BridgeOpts) {
        let li = m.nest_start(IFLA_LINKINFO);
        m.attr_str(IFLA_INFO_KIND, "bridge");
        let data = m.nest_start(IFLA_INFO_DATA);
        m.attr_u32(IFLA_BR_STP_STATE, 0);
        m.attr_u32(IFLA_BR_FORWARD_DELAY, 0);
        m.attr_u8(IFLA_BR_VLAN_FILTERING, opts.vlan_filtering as u8);
        m.attr_u16(IFLA_BR_GROUP_FWD_MASK, opts.group_fwd_mask);
        m.attr_u16(IFLA_BR_VLAN_DEFAULT_PVID, opts.default_pvid);
        m.nest_end(data);
        m.nest_end(li);
    }

    /// Create a bridge.  Fails with `EOPNOTSUPP` when the kernel has no
    /// bridge support.
    pub async fn add_bridge(&mut self, name: &str, opts: &BridgeOpts) -> io::Result<()> {
        let mut m = Msg::new(RTM_NEWLINK, NLM_F_CREATE | NLM_F_EXCL);
        m.ifinfo(0, 0, 0, 0).attr_str(IFLA_IFNAME, name);
        Self::bridge_info(&mut m, opts);
        self.ack(m).await
    }

    /// Change the options of an existing bridge.
    pub async fn set_bridge_opts(&mut self, index: u32, opts: &BridgeOpts) -> io::Result<()> {
        let mut m = Msg::new(RTM_NEWLINK, 0);
        m.ifinfo(0, index, 0, 0);
        Self::bridge_info(&mut m, opts);
        self.ack(m).await
    }

    /// Read back the options of a bridge; `Ok(None)` when `index` is not a bridge.
    pub async fn bridge_opts(&mut self, index: u32) -> io::Result<Option<BridgeOpts>> {
        let mut m = Msg::new(RTM_GETLINK, 0);
        m.ifinfo(0, index, 0, 0);
        let replies = self.request(m, RTM_NEWLINK).await?;
        let Some(p) = replies.first().filter(|p| p.len() >= 16) else {
            return Ok(None);
        };
        let mut kind = String::new();
        let mut opts = BridgeOpts::default();
        for (ty, data) in attrs(&p[16..]) {
            if ty != IFLA_LINKINFO {
                continue;
            }
            for (t, d) in attrs(data) {
                match t {
                    IFLA_INFO_KIND => kind = cstr(d),
                    IFLA_INFO_DATA => {
                        for (o, v) in attrs(d) {
                            match o {
                                IFLA_BR_VLAN_FILTERING if !v.is_empty() => {
                                    opts.vlan_filtering = v[0] != 0
                                }
                                IFLA_BR_GROUP_FWD_MASK if v.len() >= 2 => {
                                    opts.group_fwd_mask = u16_at(v, 0)
                                }
                                IFLA_BR_VLAN_DEFAULT_PVID if v.len() >= 2 => {
                                    opts.default_pvid = u16_at(v, 0)
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok((kind == "bridge").then_some(opts))
    }

    /// Create an 802.1Q device on `parent`.  The maps are `(from, to)`
    /// pairs: egress maps socket priority to the 802.1p value, ingress maps
    /// the 802.1p value to socket priority.
    pub async fn add_vlan(
        &mut self,
        name: &str,
        parent: u32,
        vid: u16,
        egress_qos: &[(u32, u32)],
        ingress_qos: &[(u32, u32)],
    ) -> io::Result<()> {
        let mut m = Msg::new(RTM_NEWLINK, NLM_F_CREATE | NLM_F_EXCL);
        m.ifinfo(0, 0, 0, 0)
            .attr_str(IFLA_IFNAME, name)
            .attr_u32(IFLA_LINK, parent);
        let li = m.nest_start(IFLA_LINKINFO);
        m.attr_str(IFLA_INFO_KIND, "vlan");
        let data = m.nest_start(IFLA_INFO_DATA);
        m.attr_u16(IFLA_VLAN_ID, vid);
        for (ty, map) in [
            (IFLA_VLAN_EGRESS_QOS, egress_qos),
            (IFLA_VLAN_INGRESS_QOS, ingress_qos),
        ] {
            if map.is_empty() {
                continue;
            }
            let n = m.nest_start(ty);
            for &(from, to) in map {
                let mut v = [0u8; 8];
                v[..4].copy_from_slice(&from.to_ne_bytes());
                v[4..].copy_from_slice(&to.to_ne_bytes());
                m.attr(IFLA_VLAN_QOS_MAPPING, &v);
            }
            m.nest_end(n);
        }
        m.nest_end(data);
        m.nest_end(li);
        self.ack(m).await
    }

    /// Create a veth pair (used by the tests).
    pub async fn add_veth(&mut self, name: &str, peer: &str) -> io::Result<()> {
        let mut m = Msg::new(RTM_NEWLINK, NLM_F_CREATE | NLM_F_EXCL);
        m.ifinfo(0, 0, 0, 0).attr_str(IFLA_IFNAME, name);
        let li = m.nest_start(IFLA_LINKINFO);
        m.attr_str(IFLA_INFO_KIND, "veth");
        let data = m.nest_start(IFLA_INFO_DATA);
        let p = m.nest_start(VETH_INFO_PEER);
        m.ifinfo(0, 0, 0, 0).attr_str(IFLA_IFNAME, peer);
        m.nest_end(p);
        m.nest_end(data);
        m.nest_end(li);
        self.ack(m).await
    }

    pub async fn del_link(&mut self, index: u32) -> io::Result<()> {
        let mut m = Msg::new(RTM_DELLINK, 0);
        m.ifinfo(0, index, 0, 0);
        self.ack(m).await
    }

    async fn bridge_vlan(
        &mut self,
        ty: u16,
        index: u32,
        vid: u16,
        flags: VlanFlags,
        on_bridge_itself: bool,
    ) -> io::Result<()> {
        let mut m = Msg::new(ty, 0);
        m.ifinfo(AF_BRIDGE, index, 0, 0);
        let spec = m.nest_start(IFLA_AF_SPEC);
        m.attr_u16(
            IFLA_BRIDGE_FLAGS,
            if on_bridge_itself {
                BRIDGE_FLAGS_SELF
            } else {
                BRIDGE_FLAGS_MASTER
            },
        );
        let f: u16 = (flags.pvid as u16) << 1 | (flags.untagged as u16) << 2;
        let mut info = [0u8; 4];
        info[..2].copy_from_slice(&f.to_ne_bytes());
        info[2..].copy_from_slice(&vid.to_ne_bytes());
        m.attr(IFLA_BRIDGE_VLAN_INFO, &info);
        m.nest_end(spec);
        self.ack(m).await
    }

    /// Add `vid` to a bridge port's VLAN table, or to the bridge device's
    /// own table (the CPU side) with `on_bridge_itself`.
    pub async fn bridge_vlan_add(
        &mut self,
        index: u32,
        vid: u16,
        flags: VlanFlags,
        on_bridge_itself: bool,
    ) -> io::Result<()> {
        self.bridge_vlan(RTM_SETLINK, index, vid, flags, on_bridge_itself)
            .await
    }

    pub async fn bridge_vlan_del(
        &mut self,
        index: u32,
        vid: u16,
        on_bridge_itself: bool,
    ) -> io::Result<()> {
        self.bridge_vlan(
            RTM_DELLINK,
            index,
            vid,
            VlanFlags::default(),
            on_bridge_itself,
        )
        .await
    }

    /// Add or replace an address.
    pub async fn addr_add(&mut self, index: u32, addr: IpAddr, prefix_len: u8) -> io::Result<()> {
        let family = if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        } as u8;
        let mut m = Msg::new(RTM_NEWADDR, NLM_F_CREATE | NLM_F_REPLACE);
        m.ifaddr(family, prefix_len, 0, index)
            .attr_ip(IFA_LOCAL, addr)
            .attr_ip(IFA_ADDRESS, addr);
        if let IpAddr::V4(a) = addr {
            if prefix_len < 31 {
                let mask = if prefix_len == 0 {
                    0
                } else {
                    u32::MAX << (32 - prefix_len as u32)
                };
                let bcast = Ipv4Addr::from(u32::from(a) | !mask);
                m.attr(IFA_BROADCAST, &bcast.octets());
            }
        }
        self.ack(m).await
    }

    pub async fn addr_del(&mut self, index: u32, addr: IpAddr, prefix_len: u8) -> io::Result<()> {
        let family = if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        } as u8;
        let mut m = Msg::new(RTM_DELADDR, 0);
        m.ifaddr(family, prefix_len, 0, index)
            .attr_ip(IFA_LOCAL, addr);
        self.ack(m).await
    }

    /// Addresses on every interface, optionally limited to one family.
    pub async fn addrs(&mut self, v6: Option<bool>) -> io::Result<Vec<Addr>> {
        let family = match v6 {
            None => 0,
            Some(false) => libc::AF_INET as u8,
            Some(true) => libc::AF_INET6 as u8,
        };
        let mut m = Msg::new(RTM_GETADDR, NLM_F_DUMP);
        m.ifaddr(family, 0, 0, 0);
        let replies = self.request(m, RTM_NEWADDR).await?;
        Ok(replies.iter().filter_map(|p| parse_addr(p)).collect())
    }

    /// Remove every IPv4 address from an interface.
    pub async fn addr_flush_v4(&mut self, index: u32) -> io::Result<()> {
        for a in self.addrs(Some(false)).await? {
            if a.index == index {
                let _ = self.addr_del(index, a.addr, a.prefix_len).await;
            }
        }
        Ok(())
    }

    /// Remove every global-scope IPv6 address from an interface, leaving
    /// the link-local address alone.
    pub async fn addr_flush_v6_global(&mut self, index: u32) -> io::Result<()> {
        let all = match self.addrs(Some(true)).await {
            Ok(a) => a,
            Err(_) => return Ok(()), // kernel without IPv6
        };
        for a in all {
            if a.index == index && a.scope == RT_SCOPE_UNIVERSE {
                let _ = self.addr_del(index, a.addr, a.prefix_len).await;
            }
        }
        Ok(())
    }

    /// Add or replace a default route through `gateway`.
    pub async fn route_add_default(
        &mut self,
        index: u32,
        gateway: IpAddr,
        metric: u32,
        protocol: u8,
    ) -> io::Result<()> {
        let family = if gateway.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        } as u8;
        let mut m = Msg::new(RTM_NEWROUTE, NLM_F_CREATE | NLM_F_REPLACE);
        m.rtmsg(
            family,
            0,
            RT_TABLE_MAIN,
            protocol,
            RT_SCOPE_UNIVERSE,
            RTN_UNICAST,
        )
        .attr_ip(RTA_GATEWAY, gateway)
        .attr_u32(RTA_OIF, index)
        .attr_u32(RTA_PRIORITY, metric);
        self.ack(m).await
    }

    /// Add or replace an on-link route to `dst/len` (used for a gateway
    /// outside the address's subnet).
    pub async fn route_add_onlink(
        &mut self,
        index: u32,
        dst: IpAddr,
        dst_len: u8,
        protocol: u8,
    ) -> io::Result<()> {
        let family = if dst.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        } as u8;
        let mut m = Msg::new(RTM_NEWROUTE, NLM_F_CREATE | NLM_F_REPLACE);
        m.rtmsg(
            family,
            dst_len,
            RT_TABLE_MAIN,
            protocol,
            RT_SCOPE_LINK,
            RTN_UNICAST,
        )
        .attr_ip(RTA_DST, dst)
        .attr_u32(RTA_OIF, index);
        self.ack(m).await
    }

    /// Routes in every table for one family.
    pub async fn routes(&mut self, v6: bool) -> io::Result<Vec<Route>> {
        let family = if v6 { libc::AF_INET6 } else { libc::AF_INET } as u8;
        let mut m = Msg::new(RTM_GETROUTE, NLM_F_DUMP);
        m.rtmsg(family, 0, 0, 0, 0, 0);
        let replies = self.request(m, RTM_NEWROUTE).await?;
        Ok(replies.iter().filter_map(|p| parse_route(p)).collect())
    }

    async fn route_del(&mut self, r: &Route) -> io::Result<()> {
        let family = if r.v6 { libc::AF_INET6 } else { libc::AF_INET } as u8;
        let mut m = Msg::new(RTM_DELROUTE, 0);
        // Scope "nowhere" matches a route of any scope on deletion.
        m.rtmsg(family, r.dst_len, r.table, r.protocol, RT_SCOPE_NOWHERE, 0);
        if let Some(d) = r.dst {
            m.attr_ip(RTA_DST, d);
        }
        if let Some(g) = r.gateway {
            m.attr_ip(RTA_GATEWAY, g);
        }
        if let Some(o) = r.oif {
            m.attr_u32(RTA_OIF, o);
        }
        if r.metric != 0 {
            m.attr_u32(RTA_PRIORITY, r.metric);
        }
        self.ack(m).await
    }

    /// Remove the main-table routes this crate installed (by protocol) for
    /// one family, on `index` or on every interface with `None`.
    pub async fn route_flush(
        &mut self,
        index: Option<u32>,
        v6: bool,
        protocols: &[u8],
    ) -> io::Result<()> {
        let routes = match self.routes(v6).await {
            Ok(r) => r,
            Err(_) if v6 => return Ok(()), // kernel without IPv6
            Err(e) => return Err(e),
        };
        for r in routes {
            if r.table == RT_TABLE_MAIN
                && protocols.contains(&r.protocol)
                && (index.is_none() || r.oif == index)
            {
                let _ = self.route_del(&r).await;
            }
        }
        Ok(())
    }
}

/// A change reported by the kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Link(Link),
    LinkRemoved(u32),
    AddrAdded(Addr),
    AddrRemoved(Addr),
    /// DNS servers from a router advertisement (RFC 8106).  A lifetime of
    /// zero withdraws them.
    Rdnss {
        index: u32,
        lifetime_secs: u32,
        servers: Vec<Ipv6Addr>,
    },
    /// Events were lost (the socket buffer overflowed); re-read the state.
    Overrun,
}

/// A subscription to link and address changes.
pub struct Monitor {
    fd: Fd,
    pending: std::collections::VecDeque<Event>,
}

impl Monitor {
    /// Subscribe.  Must be called inside a tokio runtime.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            fd: netlink_socket(
                RTMGRP_LINK | RTMGRP_IPV4_IFADDR | RTMGRP_IPV6_IFADDR | RTMGRP_ND_USEROPT,
            )?,
            pending: Default::default(),
        })
    }

    /// Wait for the next event.
    pub async fn next(&mut self) -> io::Result<Event> {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            if let Some(e) = self.pending.pop_front() {
                return Ok(e);
            }
            let n = match self.fd.recv(&mut buf).await {
                Ok(n) => n,
                Err(e) if e.raw_os_error() == Some(libc::ENOBUFS) => return Ok(Event::Overrun),
                Err(e) => return Err(e),
            };
            for (ty, _flags, _seq, payload) in messages(&buf[..n]) {
                let ev = match ty {
                    RTM_NEWLINK => parse_link(payload).map(Event::Link),
                    RTM_DELLINK => parse_link(payload).map(|l| Event::LinkRemoved(l.index)),
                    RTM_NEWADDR => parse_addr(payload).map(Event::AddrAdded),
                    RTM_DELADDR => parse_addr(payload).map(Event::AddrRemoved),
                    RTM_NEWNDUSEROPT => parse_rdnss(payload),
                    _ => None,
                };
                if let Some(ev) = ev {
                    self.pending.push_back(ev);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_layout() {
        let mut m = Msg::new(RTM_NEWLINK, NLM_F_CREATE);
        m.ifinfo(0, 7, IFF_UP, IFF_UP).attr_str(IFLA_IFNAME, "br0");
        let n = m.nest_start(IFLA_LINKINFO);
        m.attr_str(IFLA_INFO_KIND, "bridge");
        m.nest_end(n);
        let b = m.finish(42);
        assert_eq!(u32_at(&b, 0) as usize, b.len());
        assert_eq!(b.len() % 4, 0);
        assert_eq!(u32_at(&b, 8), 42);
        let (ty, flags, seq, payload) = messages(&b).next().unwrap();
        assert_eq!((ty, seq), (RTM_NEWLINK, 42));
        assert_eq!(flags, NLM_F_CREATE | NLM_F_REQUEST);
        let link = parse_link(payload).unwrap();
        assert_eq!(link.index, 7);
        assert_eq!(link.name, "br0");
        assert_eq!(link.kind.as_deref(), Some("bridge"));
        assert!(link.is_admin_up());
    }

    #[test]
    fn attr_iteration_stops_on_garbage() {
        assert_eq!(attrs(&[1, 0, 0, 0]).count(), 0);
        assert_eq!(attrs(&[0xff, 0xff, 1, 0, 0]).count(), 0);
    }
}
