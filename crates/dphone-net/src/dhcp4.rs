//! DHCPv4 client (RFC 2131/2132).
//!
//! The client only talks DHCP; it does not touch addresses or routes.  It
//! reports leases through an event channel and the caller (normally
//! [`crate::manager`]) applies them.

use std::collections::BTreeMap;
use std::io;
use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep, sleep_until, timeout_at};

use crate::packet::{self, ETH_P_ARP, ETH_P_IP, PacketSocket};
use crate::sys::{self, MacAddr};

const MAGIC: [u8; 4] = [99, 130, 83, 99];
const BOOTREQUEST: u8 = 1;
const BOOTREPLY: u8 = 2;

pub const OPT_SUBNET: u8 = 1;
pub const OPT_ROUTER: u8 = 3;
pub const OPT_DNS: u8 = 6;
pub const OPT_HOSTNAME: u8 = 12;
pub const OPT_DOMAIN: u8 = 15;
pub const OPT_MTU: u8 = 26;
pub const OPT_BROADCAST: u8 = 28;
pub const OPT_NTP: u8 = 42;
pub const OPT_VENDOR_SPECIFIC: u8 = 43;
pub const OPT_REQUESTED_IP: u8 = 50;
pub const OPT_LEASE_TIME: u8 = 51;
pub const OPT_OVERLOAD: u8 = 52;
pub const OPT_MSG_TYPE: u8 = 53;
pub const OPT_SERVER_ID: u8 = 54;
pub const OPT_PARAM_LIST: u8 = 55;
pub const OPT_MESSAGE: u8 = 56;
pub const OPT_MAX_SIZE: u8 = 57;
pub const OPT_T1: u8 = 58;
pub const OPT_T2: u8 = 59;
pub const OPT_VENDOR_CLASS: u8 = 60;
pub const OPT_CLIENT_ID: u8 = 61;
pub const OPT_TFTP_SERVER: u8 = 66;
pub const OPT_BOOTFILE: u8 = 67;
const OPT_PAD: u8 = 0;
const OPT_END: u8 = 255;

/// DHCP message types (option 53).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgType {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Decline = 4,
    Ack = 5,
    Nak = 6,
    Release = 7,
    Inform = 8,
}

impl MsgType {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Discover,
            2 => Self::Offer,
            3 => Self::Request,
            4 => Self::Decline,
            5 => Self::Ack,
            6 => Self::Nak,
            7 => Self::Release,
            8 => Self::Inform,
            _ => return None,
        })
    }
}

/// A DHCP message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub op: u8,
    pub xid: u32,
    pub secs: u16,
    pub broadcast: bool,
    pub ciaddr: Ipv4Addr,
    pub yiaddr: Ipv4Addr,
    pub siaddr: Ipv4Addr,
    pub giaddr: Ipv4Addr,
    pub chaddr: MacAddr,
    /// The fixed `sname` field, when it is not overloaded with options.
    pub sname: String,
    /// The fixed `file` field, when it is not overloaded with options.
    pub file: String,
    /// Options in ascending order; an option split across several
    /// instances (RFC 3396) is concatenated.
    pub options: BTreeMap<u8, Vec<u8>>,
}

fn parse_options(data: &[u8], out: &mut BTreeMap<u8, Vec<u8>>) {
    let mut i = 0;
    while i < data.len() {
        let code = data[i];
        if code == OPT_PAD {
            i += 1;
            continue;
        }
        if code == OPT_END || i + 1 >= data.len() {
            break;
        }
        let len = data[i + 1] as usize;
        let end = i + 2 + len;
        if end > data.len() {
            break;
        }
        out.entry(code)
            .or_default()
            .extend_from_slice(&data[i + 2..end]);
        i = end;
    }
}

fn fixed_str(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn ip_at(b: &[u8], off: usize) -> Ipv4Addr {
    Ipv4Addr::new(b[off], b[off + 1], b[off + 2], b[off + 3])
}

impl Message {
    /// A request from this client with empty options.
    pub fn request(xid: u32, chaddr: MacAddr) -> Self {
        Self {
            op: BOOTREQUEST,
            xid,
            secs: 0,
            broadcast: false,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr,
            sname: String::new(),
            file: String::new(),
            options: BTreeMap::new(),
        }
    }

    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 240 || b[236..240] != MAGIC || b[1] != 1 || b[2] != 6 {
            return None;
        }
        let mut options = BTreeMap::new();
        parse_options(&b[240..], &mut options);
        let overload = options
            .get(&OPT_OVERLOAD)
            .and_then(|v| v.first().copied())
            .unwrap_or(0);
        if overload & 1 != 0 {
            parse_options(&b[108..236], &mut options);
        }
        if overload & 2 != 0 {
            parse_options(&b[44..108], &mut options);
        }
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&b[28..34]);
        Some(Self {
            op: b[0],
            xid: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            secs: u16::from_be_bytes([b[8], b[9]]),
            broadcast: b[10] & 0x80 != 0,
            ciaddr: ip_at(b, 12),
            yiaddr: ip_at(b, 16),
            siaddr: ip_at(b, 20),
            giaddr: ip_at(b, 24),
            chaddr: MacAddr(mac),
            sname: if overload & 2 == 0 {
                fixed_str(&b[44..108])
            } else {
                String::new()
            },
            file: if overload & 1 == 0 {
                fixed_str(&b[108..236])
            } else {
                String::new()
            },
            options,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; 240];
        b[0] = self.op;
        b[1] = 1;
        b[2] = 6;
        b[4..8].copy_from_slice(&self.xid.to_be_bytes());
        b[8..10].copy_from_slice(&self.secs.to_be_bytes());
        if self.broadcast {
            b[10] = 0x80;
        }
        b[12..16].copy_from_slice(&self.ciaddr.octets());
        b[16..20].copy_from_slice(&self.yiaddr.octets());
        b[20..24].copy_from_slice(&self.siaddr.octets());
        b[24..28].copy_from_slice(&self.giaddr.octets());
        b[28..34].copy_from_slice(&self.chaddr.0);
        let sn = self.sname.as_bytes();
        b[44..44 + sn.len().min(63)].copy_from_slice(&sn[..sn.len().min(63)]);
        let f = self.file.as_bytes();
        b[108..108 + f.len().min(127)].copy_from_slice(&f[..f.len().min(127)]);
        b[236..240].copy_from_slice(&MAGIC);
        // The message type goes first, as most servers and sniffers expect.
        if let Some(v) = self.options.get(&OPT_MSG_TYPE) {
            b.extend_from_slice(&[OPT_MSG_TYPE, v.len() as u8]);
            b.extend_from_slice(v);
        }
        for (&code, v) in &self.options {
            if code == OPT_MSG_TYPE || code == OPT_PAD || code == OPT_END {
                continue;
            }
            for chunk in v.chunks(255) {
                b.extend_from_slice(&[code, chunk.len() as u8]);
                b.extend_from_slice(chunk);
            }
            if v.is_empty() {
                b.extend_from_slice(&[code, 0]);
            }
        }
        b.push(OPT_END);
        // BOOTP minimum size.
        if b.len() < 300 {
            b.resize(300, 0);
        }
        b
    }

    pub fn msg_type(&self) -> Option<MsgType> {
        self.options
            .get(&OPT_MSG_TYPE)
            .and_then(|v| v.first())
            .and_then(|&v| MsgType::from_u8(v))
    }

    pub fn set_type(&mut self, t: MsgType) -> &mut Self {
        self.options.insert(OPT_MSG_TYPE, vec![t as u8]);
        self
    }

    pub fn opt_ip(&self, code: u8) -> Option<Ipv4Addr> {
        self.options
            .get(&code)
            .filter(|v| v.len() >= 4)
            .map(|v| ip_at(v, 0))
    }

    pub fn opt_ips(&self, code: u8) -> Vec<Ipv4Addr> {
        self.options
            .get(&code)
            .map(|v| v.chunks_exact(4).map(|c| ip_at(c, 0)).collect())
            .unwrap_or_default()
    }

    pub fn opt_u32(&self, code: u8) -> Option<u32> {
        self.options
            .get(&code)
            .filter(|v| v.len() >= 4)
            .map(|v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]))
    }

    pub fn opt_str(&self, code: u8) -> Option<String> {
        self.options
            .get(&code)
            .map(|v| fixed_str(v).trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

/// What a server granted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    pub routers: Vec<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
    pub domain: Option<String>,
    pub ntp: Vec<Ipv4Addr>,
    /// Option 66, which the phone also uses as its provisioning hint.
    pub tftp_server: Option<String>,
    /// Option 67, or the fixed `file` field.
    pub bootfile: Option<String>,
    pub mtu: Option<u16>,
    pub server: Ipv4Addr,
    /// Seconds; `u32::MAX` means the lease never expires.
    pub lease_secs: u32,
    pub t1_secs: u32,
    pub t2_secs: u32,
    /// Every option the server sent, for consumers that need more.
    pub options: BTreeMap<u8, Vec<u8>>,
}

fn mask_to_prefix(mask: Ipv4Addr) -> u8 {
    u32::from(mask).leading_ones() as u8
}

/// The classful default, for servers that omit the subnet mask.
fn classful_prefix(a: Ipv4Addr) -> u8 {
    match a.octets()[0] {
        0..=127 => 8,
        128..=191 => 16,
        _ => 24,
    }
}

impl Lease {
    /// Build a lease from a DHCPACK.
    pub fn from_ack(m: &Message) -> Option<Self> {
        if m.yiaddr.is_unspecified() {
            return None;
        }
        let lease_secs = m.opt_u32(OPT_LEASE_TIME).unwrap_or(u32::MAX);
        let (mut t1, mut t2) = (m.opt_u32(OPT_T1), m.opt_u32(OPT_T2));
        if lease_secs != u32::MAX {
            let d1 = lease_secs / 2;
            let d2 = (lease_secs as u64 * 7 / 8) as u32;
            if t1.is_none_or(|v| v >= lease_secs) {
                t1 = Some(d1);
            }
            if t2.is_none_or(|v| v >= lease_secs || v < t1.unwrap_or(0)) {
                t2 = Some(d2.max(t1.unwrap_or(0)));
            }
        }
        Some(Self {
            address: m.yiaddr,
            prefix_len: m
                .opt_ip(OPT_SUBNET)
                .map(mask_to_prefix)
                .unwrap_or_else(|| classful_prefix(m.yiaddr)),
            routers: m.opt_ips(OPT_ROUTER),
            dns: m.opt_ips(OPT_DNS),
            domain: m.opt_str(OPT_DOMAIN),
            ntp: m.opt_ips(OPT_NTP),
            tftp_server: m
                .opt_str(OPT_TFTP_SERVER)
                .or_else(|| Some(m.sname.clone()).filter(|s| !s.is_empty())),
            bootfile: m
                .opt_str(OPT_BOOTFILE)
                .or_else(|| Some(m.file.clone()).filter(|s| !s.is_empty())),
            mtu: m
                .options
                .get(&OPT_MTU)
                .filter(|v| v.len() >= 2)
                .map(|v| u16::from_be_bytes([v[0], v[1]])),
            server: m.opt_ip(OPT_SERVER_ID).unwrap_or(m.siaddr),
            lease_secs,
            t1_secs: t1.unwrap_or(u32::MAX),
            t2_secs: t2.unwrap_or(u32::MAX),
            options: m.options.clone(),
        })
    }
}

/// Client settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Interface to run on (the voice VLAN device when there is one).
    pub ifname: String,
    pub mac: MacAddr,
    /// Option 60.  Stock sends `digium_<model>_<version>`.
    pub vendor_class: String,
    /// Option 12, when set.  Stock does not send one.
    pub hostname: Option<String>,
    /// A previously held address to ask for first.
    pub requested_ip: Option<Ipv4Addr>,
    /// Probe the offered address with ARP and decline it when in use
    /// (stock `network_disable_arping=0`).
    pub arp_check: bool,
    /// Socket priority for the client's own packets.
    pub priority: u32,
}

impl Config {
    /// The stock vendor class string, e.g., `digium_D60_3_0_0`.
    pub fn stock_vendor_class(model: &str, version: &str) -> String {
        format!("digium_{model}_{version}")
    }
}

/// What the client reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A lease was obtained, or a renewal changed it.
    Bound(Lease),
    /// A renewal confirmed the lease unchanged.
    Renewed(Lease),
    /// The lease is gone (expired, or the server refused it); remove the address.
    Lost,
    /// The offered address answered an ARP probe and was declined.
    Conflict(Ipv4Addr),
}

/// What the caller can ask of a running client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// The link came back: confirm the lease still holds on this network.
    LinkUp,
    /// The link went down: stop sending until `LinkUp`.
    LinkDown,
    /// Give the lease back and stop.
    Release,
}

/// Handle to a running client task.
pub struct Client {
    pub events: mpsc::Receiver<Event>,
    commands: mpsc::Sender<Command>,
    task: tokio::task::JoinHandle<()>,
}

impl Client {
    /// Start a client.  Must be called inside a tokio runtime.
    pub fn spawn(cfg: Config) -> Self {
        let (etx, erx) = mpsc::channel(8);
        let (ctx, crx) = mpsc::channel(8);
        let task = tokio::spawn(async move {
            let mut m = Machine {
                cfg,
                events: etx,
                commands: crx,
                xid: seed_xid(),
                lease: None,
                link_up: true,
            };
            m.run().await;
        });
        Self {
            events: erx,
            commands: ctx,
            task,
        }
    }

    pub async fn command(&self, c: Command) {
        let _ = self.commands.send(c).await;
    }

    /// Release the lease (best effort) and stop the task.
    pub async fn release(mut self) {
        let _ = self.commands.send(Command::Release).await;
        let _ = tokio::time::timeout(Duration::from_secs(1), &mut self.task).await;
    }

    /// Stop without releasing.
    pub fn abort(self) {
        self.task.abort();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn seed_xid() -> u32 {
    let mut b = [0u8; 4];
    // SAFETY: the pointer and length describe `b`.
    let n = unsafe { libc::getrandom(b.as_mut_ptr() as *mut libc::c_void, 4, libc::GRND_NONBLOCK) };
    if n == 4 {
        u32::from_ne_bytes(b)
    } else {
        // Early boot without entropy: the clock and the PID are enough for a transaction ID.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        t ^ std::process::id().rotate_left(16)
    }
}

/// A cheap deterministic jitter in `0..range_ms`, derived from a counter.
fn jitter_ms(seed: u32, range_ms: u64) -> u64 {
    if range_ms == 0 {
        return 0;
    }
    let x = seed
        .wrapping_mul(0x9e37_79b9)
        .rotate_left(13)
        .wrapping_mul(0x85eb_ca6b);
    x as u64 % range_ms
}

enum Step {
    /// Start over from discovery.
    Init,
    /// Verify the current lease after a link flap or at start-up.
    Reboot,
    Bound,
    Stopped,
}

struct Machine {
    cfg: Config,
    events: mpsc::Sender<Event>,
    commands: mpsc::Receiver<Command>,
    xid: u32,
    lease: Option<(Lease, Instant)>,
    link_up: bool,
}

enum Wait {
    Reply(Message),
    Timeout,
    Command(Command),
    Closed,
}

impl Machine {
    fn base(&self, t: MsgType) -> Message {
        let mut m = Message::request(self.xid, self.cfg.mac);
        m.set_type(t);
        let mut cid = vec![1u8];
        cid.extend_from_slice(&self.cfg.mac.0);
        m.options.insert(OPT_CLIENT_ID, cid);
        if !matches!(t, MsgType::Release | MsgType::Decline) {
            m.options
                .insert(OPT_MAX_SIZE, 576u16.to_be_bytes().to_vec());
            m.options.insert(
                OPT_PARAM_LIST,
                vec![
                    OPT_SUBNET,
                    OPT_ROUTER,
                    OPT_DNS,
                    OPT_HOSTNAME,
                    OPT_DOMAIN,
                    OPT_BROADCAST,
                    OPT_NTP,
                    OPT_TFTP_SERVER,
                ],
            );
            if !self.cfg.vendor_class.is_empty() {
                m.options
                    .insert(OPT_VENDOR_CLASS, self.cfg.vendor_class.as_bytes().to_vec());
            }
            if let Some(h) = &self.cfg.hostname {
                m.options.insert(OPT_HOSTNAME, h.as_bytes().to_vec());
            }
        }
        m
    }

    fn ifindex(&self) -> io::Result<u32> {
        sys::if_index(&self.cfg.ifname)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "interface is gone"))
    }

    fn raw_socket(&self) -> io::Result<PacketSocket> {
        let s = PacketSocket::new(self.ifindex()?, ETH_P_IP)?;
        let _ = s.set_priority(self.cfg.priority);
        Ok(s)
    }

    async fn send_broadcast(&self, sock: &PacketSocket, m: &Message) -> io::Result<()> {
        let pkt = packet::build_udp4(m.ciaddr, 68, Ipv4Addr::BROADCAST, 67, 0, &m.encode());
        sock.send(MacAddr::BROADCAST, &pkt).await.map(|_| ())
    }

    /// Wait for a reply to the current transaction on the raw socket, a
    /// command, or the deadline.
    async fn wait_raw(&mut self, sock: &PacketSocket, deadline: Instant) -> Wait {
        let mut buf = [0u8; 1600];
        loop {
            tokio::select! {
                r = timeout_at(deadline, sock.recv(&mut buf)) => match r {
                    Err(_) => return Wait::Timeout,
                    Ok(Err(e)) => {
                        log::warn!("dhcp4: receive on {} failed: {e}", self.cfg.ifname);
                        // The interface may have gone away; back off rather than spin.
                        sleep(Duration::from_millis(500)).await;
                        return Wait::Timeout;
                    }
                    Ok(Ok((n, _src))) => {
                        let Some(u) = packet::parse_udp4(&buf[..n]) else { continue };
                        if u.sport != 67 || u.dport != 68 {
                            continue;
                        }
                        if let Some(m) = self.accept(u.payload) {
                            return Wait::Reply(m);
                        }
                    }
                },
                c = self.commands.recv() => match c {
                    Some(c) => return Wait::Command(c),
                    None => return Wait::Closed,
                },
            }
        }
    }

    fn accept(&self, payload: &[u8]) -> Option<Message> {
        let m = Message::parse(payload)?;
        (m.op == BOOTREPLY && m.xid == self.xid && m.chaddr == self.cfg.mac).then_some(m)
    }

    async fn emit(&self, e: Event) {
        let _ = self.events.send(e).await;
    }

    async fn run(&mut self) {
        let mut step = Step::Init;
        loop {
            step = match step {
                Step::Init => self.discover().await,
                Step::Reboot => self.reboot().await,
                Step::Bound => self.bound().await,
                Step::Stopped => return,
            };
        }
    }

    /// Wait while the link is down.  Returns `false` when the client should stop.
    async fn wait_for_link(&mut self) -> bool {
        while !self.link_up {
            match self.commands.recv().await {
                Some(Command::LinkUp) => self.link_up = true,
                Some(Command::LinkDown) => {}
                Some(Command::Release) | None => return false,
            }
        }
        true
    }

    async fn drop_lease(&mut self) {
        if self.lease.take().is_some() {
            self.emit(Event::Lost).await;
        }
    }

    /// INIT and SELECTING: broadcast DISCOVER until a server offers, then
    /// request the offer.
    async fn discover(&mut self) -> Step {
        log::debug!("dhcp4: discovering on {}", self.cfg.ifname);
        self.drop_lease().await;
        let mut attempt: u32 = 0;
        let started = Instant::now();
        loop {
            if !self.wait_for_link().await {
                return Step::Stopped;
            }
            let sock = match self.raw_socket() {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("dhcp4: cannot open {}: {e}", self.cfg.ifname);
                    sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            self.xid = self.xid.wrapping_add(1);
            let mut m = self.base(MsgType::Discover);
            m.secs = started.elapsed().as_secs().min(u16::MAX as u64) as u16;
            if let Some(ip) = self.cfg.requested_ip {
                m.options.insert(OPT_REQUESTED_IP, ip.octets().to_vec());
            }
            if let Err(e) = self.send_broadcast(&sock, &m).await {
                log::debug!("dhcp4: send failed: {e}");
            }
            // 2, 4, 8, 16, then 32 seconds, each with up to a second of jitter.
            let base = 2u64 << attempt.min(4);
            let deadline =
                Instant::now() + Duration::from_millis(base * 1000 + jitter_ms(self.xid, 1000));
            attempt += 1;
            loop {
                match self.wait_raw(&sock, deadline).await {
                    Wait::Reply(offer)
                        if offer.msg_type() == Some(MsgType::Offer)
                            && !offer.yiaddr.is_unspecified() =>
                    {
                        match self.request_offer(&sock, &offer).await {
                            Some(step) => return step,
                            None => break, // try discovery again
                        }
                    }
                    Wait::Reply(_) => continue,
                    Wait::Timeout => break,
                    Wait::Command(Command::LinkDown) => {
                        self.link_up = false;
                        break;
                    }
                    Wait::Command(Command::LinkUp) => {
                        attempt = 0;
                        break;
                    }
                    Wait::Command(Command::Release) | Wait::Closed => return Step::Stopped,
                }
            }
        }
    }

    /// REQUESTING: ask for the offered address.  `None` means go back to discovery.
    async fn request_offer(&mut self, sock: &PacketSocket, offer: &Message) -> Option<Step> {
        let server = offer.opt_ip(OPT_SERVER_ID)?;
        let mut req = self.base(MsgType::Request);
        req.options
            .insert(OPT_REQUESTED_IP, offer.yiaddr.octets().to_vec());
        req.options.insert(OPT_SERVER_ID, server.octets().to_vec());
        for attempt in 0..3u32 {
            if let Err(e) = self.send_broadcast(sock, &req).await {
                log::debug!("dhcp4: send failed: {e}");
            }
            let deadline = Instant::now() + Duration::from_secs(2 << attempt);
            loop {
                match self.wait_raw(sock, deadline).await {
                    Wait::Reply(m) => match m.msg_type() {
                        Some(MsgType::Ack) => return self.accept_ack(sock, &m, true).await,
                        Some(MsgType::Nak) => {
                            log::info!(
                                "dhcp4: server refused the request: {}",
                                m.opt_str(OPT_MESSAGE).unwrap_or_default()
                            );
                            self.cfg.requested_ip = None;
                            sleep(Duration::from_secs(1)).await;
                            return None;
                        }
                        _ => continue,
                    },
                    Wait::Timeout => break,
                    Wait::Command(Command::LinkDown) => {
                        self.link_up = false;
                        return None;
                    }
                    Wait::Command(Command::LinkUp) => continue,
                    Wait::Command(Command::Release) | Wait::Closed => return Some(Step::Stopped),
                }
            }
        }
        None
    }

    /// Handle a DHCPACK: optionally probe the address, then report the lease.
    async fn accept_ack(
        &mut self,
        sock: &PacketSocket,
        ack: &Message,
        probe: bool,
    ) -> Option<Step> {
        let lease = Lease::from_ack(ack)?;
        if probe && self.cfg.arp_check && self.address_in_use(lease.address).await {
            log::warn!("dhcp4: {} is already in use; declining", lease.address);
            let mut d = self.base(MsgType::Decline);
            d.options
                .insert(OPT_REQUESTED_IP, lease.address.octets().to_vec());
            d.options
                .insert(OPT_SERVER_ID, lease.server.octets().to_vec());
            let _ = self.send_broadcast(sock, &d).await;
            self.emit(Event::Conflict(lease.address)).await;
            self.cfg.requested_ip = None;
            sleep(Duration::from_secs(10)).await;
            return None;
        }
        let previous = self.lease.as_ref().map(|(l, _)| l.clone());
        self.cfg.requested_ip = Some(lease.address);
        self.lease = Some((lease.clone(), Instant::now()));
        if previous.as_ref().is_some_and(|p| same_config(p, &lease)) {
            self.emit(Event::Renewed(lease)).await;
        } else {
            self.emit(Event::Bound(lease)).await;
        }
        Some(Step::Bound)
    }

    /// ARP-probe `ip` (RFC 5227 style: sender address zero).  True when
    /// some other host answers.
    async fn address_in_use(&self, ip: Ipv4Addr) -> bool {
        let Ok(idx) = self.ifindex() else {
            return false;
        };
        let Ok(sock) = PacketSocket::new(idx, ETH_P_ARP) else {
            return false;
        };
        let probe = arp_probe(self.cfg.mac, ip);
        let mut buf = [0u8; 128];
        for _ in 0..2 {
            if sock.send(MacAddr::BROADCAST, &probe).await.is_err() {
                return false;
            }
            let deadline = Instant::now() + Duration::from_millis(500);
            while let Ok(Ok((n, _))) = timeout_at(deadline, sock.recv(&mut buf)).await {
                if arp_conflicts(&buf[..n], self.cfg.mac, ip) {
                    return true;
                }
            }
        }
        false
    }

    /// BOUND, RENEWING, and REBINDING.
    async fn bound(&mut self) -> Step {
        let Some((lease, obtained)) = self.lease.clone() else {
            return Step::Init;
        };
        if lease.lease_secs == u32::MAX {
            // Infinite lease: only commands can move us.
            loop {
                match self.commands.recv().await {
                    Some(Command::LinkDown) => self.link_up = false,
                    Some(Command::LinkUp) => {
                        self.link_up = true;
                        return Step::Reboot;
                    }
                    Some(Command::Release) => return self.release().await,
                    None => return Step::Stopped,
                }
            }
        }
        let t1 = obtained + Duration::from_secs(lease.t1_secs as u64);
        let t2 = obtained + Duration::from_secs(lease.t2_secs as u64);
        let expiry = obtained + Duration::from_secs(lease.lease_secs as u64);

        // BOUND: sleep until T1.
        loop {
            tokio::select! {
                _ = sleep_until(t1) => break,
                c = self.commands.recv() => match c {
                    Some(Command::LinkDown) => self.link_up = false,
                    Some(Command::LinkUp) => { self.link_up = true; return Step::Reboot; }
                    Some(Command::Release) => return self.release().await,
                    None => return Step::Stopped,
                },
            }
        }

        // RENEWING: unicast to the leasing server until T2.
        log::debug!("dhcp4: renewing {} with {}", lease.address, lease.server);
        self.xid = self.xid.wrapping_add(1);
        let mut req = self.base(MsgType::Request);
        req.ciaddr = lease.address;
        if let Ok(udp) = self.renew_socket(lease.address) {
            let mut buf = [0u8; 1600];
            while Instant::now() < t2 {
                if self.link_up {
                    if let Err(e) = udp.send_to(&req.encode(), (lease.server, 67)).await {
                        log::debug!("dhcp4: renewal send failed: {e}");
                    }
                }
                // Retransmit after half the time left to T2, at least 60 seconds.
                let left = t2.saturating_duration_since(Instant::now());
                let deadline = (Instant::now() + (left / 2).max(Duration::from_secs(60))).min(t2);
                loop {
                    tokio::select! {
                        r = timeout_at(deadline, udp.recv_from(&mut buf)) => match r {
                            Err(_) => break,
                            Ok(Err(_)) => { sleep(Duration::from_secs(1)).await; break; }
                            Ok(Ok((n, _))) => {
                                let Some(m) = self.accept(&buf[..n]) else { continue };
                                match m.msg_type() {
                                    Some(MsgType::Ack) => {
                                        if let Some(l) = Lease::from_ack(&m) {
                                            return self.renewed(l).await;
                                        }
                                    }
                                    Some(MsgType::Nak) => return Step::Init,
                                    _ => {}
                                }
                            }
                        },
                        c = self.commands.recv() => match c {
                            Some(Command::LinkDown) => self.link_up = false,
                            Some(Command::LinkUp) => { self.link_up = true; return Step::Reboot; }
                            Some(Command::Release) => return self.release().await,
                            None => return Step::Stopped,
                        },
                    }
                }
            }
        } else {
            log::warn!("dhcp4: cannot open the renewal socket; waiting for the rebinding time");
            tokio::select! {
                _ = sleep_until(t2) => {}
                c = self.commands.recv() => match c {
                    Some(Command::Release) => return self.release().await,
                    Some(Command::LinkUp) => { self.link_up = true; return Step::Reboot; }
                    Some(Command::LinkDown) => self.link_up = false,
                    None => return Step::Stopped,
                },
            }
        }

        // REBINDING: broadcast until the lease runs out.
        log::debug!("dhcp4: rebinding {}", lease.address);
        match self.rebind(&req, expiry).await {
            Some(step) => step,
            None => Step::Init,
        }
    }

    async fn rebind(&mut self, req: &Message, expiry: Instant) -> Option<Step> {
        while Instant::now() < expiry {
            let Ok(sock) = self.raw_socket() else {
                sleep(Duration::from_secs(2)).await;
                continue;
            };
            if self.link_up {
                let _ = self.send_broadcast(&sock, req).await;
            }
            let left = expiry.saturating_duration_since(Instant::now());
            let deadline = (Instant::now() + (left / 2).max(Duration::from_secs(60))).min(expiry);
            loop {
                match self.wait_raw(&sock, deadline).await {
                    Wait::Reply(m) => match m.msg_type() {
                        Some(MsgType::Ack) => {
                            if let Some(l) = Lease::from_ack(&m) {
                                return Some(self.renewed(l).await);
                            }
                        }
                        Some(MsgType::Nak) => return None,
                        _ => {}
                    },
                    Wait::Timeout => break,
                    Wait::Command(Command::LinkDown) => self.link_up = false,
                    Wait::Command(Command::LinkUp) => {
                        self.link_up = true;
                        return Some(Step::Reboot);
                    }
                    Wait::Command(Command::Release) => return Some(self.release().await),
                    Wait::Closed => return Some(Step::Stopped),
                }
            }
        }
        None
    }

    async fn renewed(&mut self, lease: Lease) -> Step {
        let changed = self
            .lease
            .as_ref()
            .is_none_or(|(old, _)| !same_config(old, &lease));
        self.lease = Some((lease.clone(), Instant::now()));
        self.emit(if changed {
            Event::Bound(lease)
        } else {
            Event::Renewed(lease)
        })
        .await;
        Step::Bound
    }

    /// INIT-REBOOT: after a link flap, ask whether the address is still
    /// good on this network.  No answer keeps the lease; a NAK drops it.
    async fn reboot(&mut self) -> Step {
        let Some((lease, obtained)) = self.lease.clone() else {
            return Step::Init;
        };
        log::debug!("dhcp4: confirming {} after a link change", lease.address);
        if lease.lease_secs != u32::MAX
            && obtained.elapsed() >= Duration::from_secs(lease.lease_secs as u64)
        {
            return Step::Init;
        }
        let Ok(sock) = self.raw_socket() else {
            return Step::Bound;
        };
        self.xid = self.xid.wrapping_add(1);
        let mut req = self.base(MsgType::Request);
        req.options
            .insert(OPT_REQUESTED_IP, lease.address.octets().to_vec());
        for attempt in 0..3u32 {
            let _ = self.send_broadcast(&sock, &req).await;
            let deadline = Instant::now() + Duration::from_secs(2 << attempt);
            loop {
                match self.wait_raw(&sock, deadline).await {
                    Wait::Reply(m) => match m.msg_type() {
                        Some(MsgType::Ack) => {
                            if let Some(l) = Lease::from_ack(&m) {
                                return self.renewed(l).await;
                            }
                        }
                        Some(MsgType::Nak) => return Step::Init,
                        _ => {}
                    },
                    Wait::Timeout => break,
                    Wait::Command(Command::LinkDown) => {
                        self.link_up = false;
                        return Step::Bound;
                    }
                    Wait::Command(Command::LinkUp) => {}
                    Wait::Command(Command::Release) => return self.release().await,
                    Wait::Closed => return Step::Stopped,
                }
            }
        }
        // Nobody confirmed the address, so this may be a different network:
        // drop the lease and start over.
        Step::Init
    }

    fn renew_socket(&self, addr: Ipv4Addr) -> io::Result<UdpSocket> {
        let fd = sys::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)?;
        use std::os::fd::AsRawFd;
        sys::setsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_REUSEADDR, &1i32)?;
        sys::setsockopt_bytes(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            self.cfg.ifname.as_bytes(),
        )?;
        let _ = sys::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PRIORITY,
            &(self.cfg.priority as i32),
        );
        let std_sock = std::net::UdpSocket::from(fd);
        // Bind after the options so the device binding applies.
        let sa = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: 68u16.to_be(),
            sin_addr: libc::in_addr {
                s_addr: u32::from(addr).to_be(),
            },
            sin_zero: [0; 8],
        };
        // SAFETY: `sa` is a valid `sockaddr_in` of the length passed.
        sys::cvt(unsafe {
            libc::bind(
                std_sock.as_raw_fd(),
                &sa as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        })?;
        UdpSocket::from_std(std_sock)
    }

    async fn release(&mut self) -> Step {
        if let Some((lease, _)) = self.lease.take() {
            self.xid = self.xid.wrapping_add(1);
            let mut m = self.base(MsgType::Release);
            m.ciaddr = lease.address;
            m.options
                .insert(OPT_SERVER_ID, lease.server.octets().to_vec());
            if let Ok(udp) = self.renew_socket(lease.address) {
                let _ = udp.send_to(&m.encode(), (lease.server, 67)).await;
            }
        }
        Step::Stopped
    }
}

/// Whether two leases configure the interface identically.
fn same_config(a: &Lease, b: &Lease) -> bool {
    a.address == b.address
        && a.prefix_len == b.prefix_len
        && a.routers == b.routers
        && a.dns == b.dns
        && a.domain == b.domain
        && a.ntp == b.ntp
        && a.tftp_server == b.tftp_server
        && a.mtu == b.mtu
}

/// An ARP probe for `target`: a request with a zero sender address.
pub(crate) fn arp_probe(mac: MacAddr, target: Ipv4Addr) -> Vec<u8> {
    let mut p = vec![0, 1, 8, 0, 6, 4, 0, 1];
    p.extend_from_slice(&mac.0);
    p.extend_from_slice(&[0; 4]);
    p.extend_from_slice(&[0; 6]);
    p.extend_from_slice(&target.octets());
    p
}

/// Whether an ARP packet shows another host using `ip`.
pub(crate) fn arp_conflicts(p: &[u8], our_mac: MacAddr, ip: Ipv4Addr) -> bool {
    if p.len() < 28 || p[0..6] != [0, 1, 8, 0, 6, 4] {
        return false;
    }
    let sender_mac = &p[8..14];
    let sender_ip = &p[14..18];
    sender_mac != our_mac.0 && sender_ip == ip.octets()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DHCPACK built by hand, with the field values of the DHCPACK in the
    /// Wireshark `dhcp.pcap` sample capture.
    fn sample_ack() -> Vec<u8> {
        let mut b = vec![0u8; 240];
        b[0] = 2;
        b[1] = 1;
        b[2] = 6;
        b[4..8].copy_from_slice(&0x0000_3d1eu32.to_be_bytes());
        b[16..20].copy_from_slice(&[192, 168, 0, 10]);
        b[28..34].copy_from_slice(&[0x00, 0x0b, 0x82, 0x01, 0xfc, 0x42]);
        b[236..240].copy_from_slice(&MAGIC);
        b.extend_from_slice(&[53, 1, 5]);
        b.extend_from_slice(&[58, 4, 0, 0, 0x07, 0x08]);
        b.extend_from_slice(&[59, 4, 0, 0, 0x0c, 0x4e]);
        b.extend_from_slice(&[51, 4, 0, 0, 0x0e, 0x10]);
        b.extend_from_slice(&[54, 4, 192, 168, 0, 1]);
        b.extend_from_slice(&[1, 4, 255, 255, 255, 0]);
        b.push(255);
        b
    }

    #[test]
    fn parses_sample_ack() {
        let m = Message::parse(&sample_ack()).unwrap();
        assert_eq!(m.msg_type(), Some(MsgType::Ack));
        assert_eq!(m.xid, 0x3d1e);
        let l = Lease::from_ack(&m).unwrap();
        assert_eq!(l.address, Ipv4Addr::new(192, 168, 0, 10));
        assert_eq!(l.prefix_len, 24);
        assert_eq!(l.server, Ipv4Addr::new(192, 168, 0, 1));
        assert_eq!((l.lease_secs, l.t1_secs, l.t2_secs), (3600, 1800, 3150));
    }

    #[test]
    fn encode_round_trip_and_option_order() {
        let mut m = Message::request(0xdead_beef, MacAddr([0, 0x0f, 0xd3, 1, 2, 3]));
        m.set_type(MsgType::Discover);
        m.options
            .insert(OPT_VENDOR_CLASS, b"digium_D60_3_0_0".to_vec());
        m.options.insert(OPT_PARAM_LIST, vec![1, 3, 6]);
        let b = m.encode();
        assert!(b.len() >= 300);
        assert_eq!(&b[240..243], &[53, 1, 1]);
        let back = Message::parse(&b).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn long_options_split_and_rejoin() {
        let mut m = Message::request(1, MacAddr([2, 0, 0, 0, 0, 1]));
        m.set_type(MsgType::Ack);
        m.options.insert(43, (0..600u32).map(|i| i as u8).collect());
        let back = Message::parse(&m.encode()).unwrap();
        assert_eq!(back.options[&43].len(), 600);
        assert_eq!(back.options[&43], m.options[&43]);
    }

    #[test]
    fn overloaded_fields() {
        let mut b = sample_ack();
        // Overload `file` with options: option 66 lives there.
        let end = b.len() - 1;
        b.truncate(end);
        b.extend_from_slice(&[52, 1, 1, 255]);
        let opt = [
            66, 9, b's', b'i', b'p', b':', b'p', b'b', b'x', b'.', b'x', 255,
        ];
        b[108..108 + opt.len()].copy_from_slice(&opt);
        let m = Message::parse(&b).unwrap();
        assert_eq!(m.file, "");
        assert_eq!(
            Lease::from_ack(&m).unwrap().tftp_server.as_deref(),
            Some("sip:pbx.x")
        );
    }

    #[test]
    fn lease_defaults() {
        let mut m = Message::request(1, MacAddr([2, 0, 0, 0, 0, 1]));
        m.op = BOOTREPLY;
        m.set_type(MsgType::Ack);
        m.yiaddr = Ipv4Addr::new(10, 1, 2, 3);
        m.options
            .insert(OPT_LEASE_TIME, 800u32.to_be_bytes().to_vec());
        let l = Lease::from_ack(&m).unwrap();
        assert_eq!(l.prefix_len, 8);
        assert_eq!((l.t1_secs, l.t2_secs), (400, 700));
        m.options.remove(&OPT_LEASE_TIME);
        assert_eq!(Lease::from_ack(&m).unwrap().lease_secs, u32::MAX);
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(Message::parse(&[0u8; 100]).is_none());
        let mut b = sample_ack();
        b[236] = 0;
        assert!(Message::parse(&b).is_none());
    }

    #[test]
    fn arp_probe_and_conflict() {
        let me = MacAddr([2, 0, 0, 0, 0, 1]);
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        let p = arp_probe(me, ip);
        assert_eq!(p.len(), 28);
        assert!(!arp_conflicts(&p, me, ip));
        let mut reply = vec![0, 1, 8, 0, 6, 4, 0, 2];
        reply.extend_from_slice(&[2, 0, 0, 0, 0, 9]);
        reply.extend_from_slice(&ip.octets());
        reply.extend_from_slice(&me.0);
        reply.extend_from_slice(&[0; 4]);
        assert!(arp_conflicts(&reply, me, ip));
    }
}
