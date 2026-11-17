//! DHCPv6 client (RFC 8415): stateful address assignment (one IA_NA) with a
//! fall-back to stateless information requests when no server hands out
//! addresses.  SLAAC itself is left to the kernel.
//!
//! As with DHCPv4, the client only talks the protocol and reports results.

use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::os::fd::AsRawFd;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep, timeout_at};

use crate::sys::{self, MacAddr};

const ALL_SERVERS: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 1, 2);

const SOLICIT: u8 = 1;
const ADVERTISE: u8 = 2;
const REQUEST: u8 = 3;
const RENEW: u8 = 5;
const REBIND: u8 = 6;
const REPLY: u8 = 7;
const RELEASE: u8 = 8;
const INFORMATION_REQUEST: u8 = 11;

const OPT_CLIENTID: u16 = 1;
const OPT_SERVERID: u16 = 2;
const OPT_IA_NA: u16 = 3;
const OPT_IAADDR: u16 = 5;
const OPT_ORO: u16 = 6;
const OPT_ELAPSED: u16 = 8;
const OPT_STATUS: u16 = 13;
const OPT_USER_CLASS: u16 = 15;
const OPT_DNS: u16 = 23;
const OPT_DOMAIN_LIST: u16 = 24;
const OPT_SNTP: u16 = 31;
const OPT_INFO_REFRESH: u16 = 32;

/// A DHCPv6 message: type, transaction ID, and options in wire order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub msg_type: u8,
    pub xid: u32,
    pub options: Vec<(u16, Vec<u8>)>,
}

fn parse_opts(mut b: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut out = Vec::new();
    while b.len() >= 4 {
        let code = u16::from_be_bytes([b[0], b[1]]);
        let len = u16::from_be_bytes([b[2], b[3]]) as usize;
        if b.len() < 4 + len {
            break;
        }
        out.push((code, b[4..4 + len].to_vec()));
        b = &b[4 + len..];
    }
    out
}

fn put_opt(out: &mut Vec<u8>, code: u16, data: &[u8]) {
    out.extend_from_slice(&code.to_be_bytes());
    out.extend_from_slice(&(data.len() as u16).to_be_bytes());
    out.extend_from_slice(data);
}

impl Message {
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 4 {
            return None;
        }
        Some(Self {
            msg_type: b[0],
            xid: u32::from_be_bytes([0, b[1], b[2], b[3]]),
            options: parse_opts(&b[4..]),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![
            self.msg_type,
            (self.xid >> 16) as u8,
            (self.xid >> 8) as u8,
            self.xid as u8,
        ];
        for (code, data) in &self.options {
            put_opt(&mut out, *code, data);
        }
        out
    }

    pub fn opt(&self, code: u16) -> Option<&[u8]> {
        self.options
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, d)| d.as_slice())
    }
}

/// One assigned address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Address {
    pub addr: Ipv6Addr,
    pub preferred_secs: u32,
    pub valid_secs: u32,
}

/// What a server granted or told us.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lease {
    /// Empty for a stateless (information-only) result.
    pub addresses: Vec<Address>,
    pub dns: Vec<Ipv6Addr>,
    pub domains: Vec<String>,
    pub sntp: Vec<Ipv6Addr>,
    pub t1_secs: u32,
    pub t2_secs: u32,
    pub server_id: Vec<u8>,
}

fn addrs16(d: &[u8]) -> Vec<Ipv6Addr> {
    d.chunks_exact(16)
        .map(|c| {
            let mut o = [0u8; 16];
            o.copy_from_slice(c);
            Ipv6Addr::from(o)
        })
        .collect()
}

fn domain_list(mut d: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    while let Some(&len) = d.first() {
        let len = len as usize;
        d = &d[1..];
        if len == 0 {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if len > 63 || d.len() < len {
            break;
        }
        if !cur.is_empty() {
            cur.push('.');
        }
        cur.push_str(&String::from_utf8_lossy(&d[..len]));
        d = &d[len..];
    }
    out
}

fn status_code(opts: &[(u16, Vec<u8>)]) -> u16 {
    opts.iter()
        .find(|(c, _)| *c == OPT_STATUS)
        .filter(|(_, d)| d.len() >= 2)
        .map(|(_, d)| u16::from_be_bytes([d[0], d[1]]))
        .unwrap_or(0)
}

impl Lease {
    /// Extract the lease for `iaid` from an Advertise or Reply.  Returns
    /// `None` when the message carries an error status.
    pub fn from_message(m: &Message, iaid: u32) -> Option<Self> {
        if status_code(&m.options) != 0 {
            return None;
        }
        let mut l = Lease {
            server_id: m.opt(OPT_SERVERID).unwrap_or_default().to_vec(),
            ..Default::default()
        };
        for (code, d) in &m.options {
            match *code {
                OPT_DNS => l.dns = addrs16(d),
                OPT_SNTP => l.sntp = addrs16(d),
                OPT_DOMAIN_LIST => l.domains = domain_list(d),
                OPT_IA_NA
                    if d.len() >= 12 && u32::from_be_bytes([d[0], d[1], d[2], d[3]]) == iaid =>
                {
                    let sub = parse_opts(&d[12..]);
                    if status_code(&sub) != 0 {
                        continue;
                    }
                    l.t1_secs = u32::from_be_bytes([d[4], d[5], d[6], d[7]]);
                    l.t2_secs = u32::from_be_bytes([d[8], d[9], d[10], d[11]]);
                    for (c, a) in &sub {
                        if *c == OPT_IAADDR
                            && a.len() >= 24
                            && status_code(&parse_opts(&a[24..])) == 0
                        {
                            let valid = u32::from_be_bytes([a[20], a[21], a[22], a[23]]);
                            if valid != 0 {
                                l.addresses.push(Address {
                                    addr: addrs16(&a[..16])[0],
                                    preferred_secs: u32::from_be_bytes([
                                        a[16], a[17], a[18], a[19],
                                    ]),
                                    valid_secs: valid,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !l.addresses.is_empty() {
            let min_pref = l
                .addresses
                .iter()
                .map(|a| a.preferred_secs)
                .min()
                .unwrap_or(0);
            if l.t1_secs == 0 {
                l.t1_secs = if min_pref == u32::MAX {
                    u32::MAX
                } else {
                    min_pref / 2
                };
            }
            if l.t2_secs == 0 {
                l.t2_secs = if min_pref == u32::MAX {
                    u32::MAX
                } else {
                    (min_pref as u64 * 4 / 5) as u32
                };
            }
        }
        Some(l)
    }

    fn min_valid(&self) -> u32 {
        self.addresses
            .iter()
            .map(|a| a.valid_secs)
            .min()
            .unwrap_or(0)
    }
}

/// Client settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub ifname: String,
    pub mac: MacAddr,
    /// Sent as the user class, as stock's `odhcp6c -u` does.
    pub user_class: String,
}

/// What the client reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Addresses (possibly none, for stateless operation) and options.
    Bound(Lease),
    /// The lease ran out; remove its addresses.
    Lost,
}

/// Handle to a running client task.
pub struct Client {
    pub events: mpsc::Receiver<Event>,
    restart: mpsc::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl Client {
    /// Start a client.  Must be called inside a tokio runtime.
    pub fn spawn(cfg: Config) -> Self {
        let (etx, erx) = mpsc::channel(8);
        let (rtx, rrx) = mpsc::channel(2);
        let task = tokio::spawn(run(cfg, etx, rrx));
        Self {
            events: erx,
            restart: rtx,
            task,
        }
    }

    /// The link came back: start the exchange over.
    pub async fn link_up(&self) {
        let _ = self.restart.send(()).await;
    }

    pub fn abort(self) {
        self.task.abort();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn duid(mac: MacAddr) -> Vec<u8> {
    // DUID-LL: type 3, hardware type 1 (Ethernet).
    let mut d = vec![0, 3, 0, 1];
    d.extend_from_slice(&mac.0);
    d
}

fn iaid(mac: MacAddr) -> u32 {
    u32::from_be_bytes([mac.0[2], mac.0[3], mac.0[4], mac.0[5]])
}

/// Build a client message.  `lease` supplies the server ID and addresses
/// for Request, Renew, Rebind, and Release.
pub fn build(
    msg_type: u8,
    xid: u32,
    cfg: &Config,
    elapsed_cs: u16,
    lease: Option<&Lease>,
) -> Message {
    let mut m = Message {
        msg_type,
        xid: xid & 0xff_ffff,
        options: Vec::new(),
    };
    m.options.push((OPT_CLIENTID, duid(cfg.mac)));
    if let Some(l) = lease {
        if msg_type != REBIND && !l.server_id.is_empty() {
            m.options.push((OPT_SERVERID, l.server_id.clone()));
        }
    }
    m.options
        .push((OPT_ELAPSED, elapsed_cs.to_be_bytes().to_vec()));
    if msg_type != RELEASE {
        let mut oro = Vec::new();
        for o in [OPT_DNS, OPT_DOMAIN_LIST, OPT_SNTP, OPT_INFO_REFRESH] {
            oro.extend_from_slice(&o.to_be_bytes());
        }
        m.options.push((OPT_ORO, oro));
        if !cfg.user_class.is_empty() {
            let mut uc = (cfg.user_class.len() as u16).to_be_bytes().to_vec();
            uc.extend_from_slice(cfg.user_class.as_bytes());
            m.options.push((OPT_USER_CLASS, uc));
        }
    }
    if msg_type != INFORMATION_REQUEST {
        let mut ia = iaid(cfg.mac).to_be_bytes().to_vec();
        ia.extend_from_slice(&[0; 8]); // T1 and T2 left to the server
        if let Some(l) = lease {
            for a in &l.addresses {
                let mut sub = a.addr.octets().to_vec();
                sub.extend_from_slice(&[0; 8]);
                put_opt(&mut ia, OPT_IAADDR, &sub);
            }
        }
        m.options.push((OPT_IA_NA, ia));
    }
    m
}

struct Session {
    cfg: Config,
    sock: UdpSocket,
    dest: SocketAddr,
    xid: u32,
}

impl Session {
    fn open(cfg: &Config) -> io::Result<Self> {
        let index = sys::if_index(&cfg.ifname)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such interface"))?;
        let fd = sys::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0)?;
        let raw = fd.as_raw_fd();
        sys::setsockopt(raw, libc::SOL_SOCKET, libc::SO_REUSEADDR, &1i32)?;
        sys::setsockopt(raw, libc::IPPROTO_IPV6, libc::IPV6_V6ONLY, &1i32)?;
        sys::setsockopt_bytes(
            raw,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            cfg.ifname.as_bytes(),
        )?;
        sys::setsockopt(
            raw,
            libc::IPPROTO_IPV6,
            libc::IPV6_MULTICAST_IF,
            &(index as i32),
        )?;
        let std_sock = std::net::UdpSocket::from(fd);
        // SAFETY: zero is a valid bit pattern for `sockaddr_in6`.
        let mut sa: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
        sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
        sa.sin6_port = 546u16.to_be();
        // SAFETY: `sa` is a valid `sockaddr_in6` of the length passed.
        sys::cvt(unsafe {
            libc::bind(
                raw,
                &sa as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
            )
        })?;
        let mut xid = [0u8; 4];
        // SAFETY: the pointer and length describe `xid`.
        unsafe {
            libc::getrandom(
                xid.as_mut_ptr() as *mut libc::c_void,
                4,
                libc::GRND_NONBLOCK,
            )
        };
        Ok(Self {
            cfg: cfg.clone(),
            sock: UdpSocket::from_std(std_sock)?,
            dest: SocketAddr::V6(SocketAddrV6::new(ALL_SERVERS, 547, 0, index)),
            xid: u32::from_ne_bytes(xid) ^ std::process::id(),
        })
    }

    /// Send `msg_type` with RFC 8415 retransmission (`irt`, `mrt`, at most
    /// `mrc` tries when non-zero, until `deadline` when given) and return
    /// the first reply of type `want` that `accept` approves.
    #[allow(clippy::too_many_arguments)]
    async fn exchange(
        &mut self,
        msg_type: u8,
        want: u8,
        lease: Option<&Lease>,
        irt: Duration,
        mrt: Duration,
        mrc: u32,
        deadline: Option<Instant>,
        accept: impl Fn(&Message) -> bool,
    ) -> Option<Message> {
        self.xid = self.xid.wrapping_add(1) & 0xff_ffff;
        let started = Instant::now();
        let mut rt = irt;
        let mut tries = 0;
        let mut buf = [0u8; 1500];
        loop {
            if mrc != 0 && tries >= mrc {
                return None;
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return None;
            }
            tries += 1;
            let elapsed = (started.elapsed().as_millis() / 10).min(0xffff) as u16;
            let m = build(msg_type, self.xid, &self.cfg, elapsed, lease);
            if let Err(e) = self.sock.send_to(&m.encode(), self.dest).await {
                // Typically the link-local address is still tentative.
                log::debug!("dhcp6: send failed: {e}");
            }
            let mut until = Instant::now() + rt;
            if let Some(d) = deadline {
                until = until.min(d);
            }
            while let Ok(r) = timeout_at(until, self.sock.recv_from(&mut buf)).await {
                let Ok((n, _)) = r else { break };
                let Some(reply) = Message::parse(&buf[..n]) else {
                    continue;
                };
                if reply.msg_type == want
                    && reply.xid == self.xid
                    && reply.opt(OPT_CLIENTID) == Some(duid(self.cfg.mac).as_slice())
                    && accept(&reply)
                {
                    return Some(reply);
                }
            }
            rt = (rt * 2).min(mrt);
        }
    }
}

async fn run(cfg: Config, events: mpsc::Sender<Event>, mut restart: mpsc::Receiver<()>) {
    let iaid = iaid(cfg.mac);
    loop {
        // Run one acquisition-and-hold cycle; a link flap restarts it.
        tokio::select! {
            _ = cycle(&cfg, iaid, &events) => {}
            r = restart.recv() => {
                if r.is_none() {
                    return;
                }
            }
        }
    }
}

async fn cycle(cfg: &Config, iaid: u32, events: &mpsc::Sender<Event>) {
    let mut s = match Session::open(cfg) {
        Ok(s) => s,
        Err(e) => {
            // No IPv6 in the kernel, or the interface is gone.
            log::debug!("dhcp6: cannot open a socket on {}: {e}", cfg.ifname);
            sleep(Duration::from_secs(60)).await;
            return;
        }
    };
    // Give duplicate address detection time to finish on the link-local address.
    sleep(Duration::from_millis(1500)).await;

    // Stateful first: four Solicits (about 15 seconds) looking for a server
    // that offers an address.
    let has_addr =
        |m: &Message| Lease::from_message(m, iaid).is_some_and(|l| !l.addresses.is_empty());
    let adv = s
        .exchange(
            SOLICIT,
            ADVERTISE,
            None,
            Duration::from_secs(1),
            Duration::from_secs(8),
            4,
            None,
            has_addr,
        )
        .await;
    let Some(adv) = adv else {
        // Stateless: ask for DNS and friends only.
        let reply = s
            .exchange(
                INFORMATION_REQUEST,
                REPLY,
                None,
                Duration::from_secs(1),
                Duration::from_secs(120),
                6,
                None,
                |m| status_code(&m.options) == 0,
            )
            .await;
        match reply {
            Some(m) => {
                let lease = Lease::from_message(&m, iaid).unwrap_or_default();
                let refresh = m
                    .opt(OPT_INFO_REFRESH)
                    .filter(|d| d.len() >= 4)
                    .map(|d| u32::from_be_bytes([d[0], d[1], d[2], d[3]]))
                    .unwrap_or(86_400)
                    .clamp(600, 86_400);
                let _ = events.send(Event::Bound(lease)).await;
                sleep(Duration::from_secs(refresh as u64)).await;
            }
            // No DHCPv6 service at all; look again later.
            None => sleep(Duration::from_secs(600)).await,
        }
        return;
    };

    let offered = Lease::from_message(&adv, iaid).expect("checked by the accept closure");
    let reply = s
        .exchange(
            REQUEST,
            REPLY,
            Some(&offered),
            Duration::from_secs(1),
            Duration::from_secs(30),
            10,
            None,
            has_addr,
        )
        .await;
    let Some(reply) = reply else { return };
    let mut lease = Lease::from_message(&reply, iaid).expect("checked by the accept closure");
    let _ = events.send(Event::Bound(lease.clone())).await;

    loop {
        let obtained = Instant::now();
        let secs = |v: u32| Duration::from_secs(v as u64);
        let expiry = obtained + secs(lease.min_valid());
        if lease.t1_secs == u32::MAX || lease.min_valid() == u32::MAX {
            std::future::pending::<()>().await;
        }
        tokio::time::sleep_until((obtained + secs(lease.t1_secs)).min(expiry)).await;
        let t2 = (obtained + secs(lease.t2_secs.max(lease.t1_secs))).min(expiry);
        let mut renewed = s
            .exchange(
                RENEW,
                REPLY,
                Some(&lease),
                Duration::from_secs(10),
                Duration::from_secs(600),
                0,
                Some(t2),
                has_addr,
            )
            .await;
        if renewed.is_none() {
            renewed = s
                .exchange(
                    REBIND,
                    REPLY,
                    Some(&lease),
                    Duration::from_secs(10),
                    Duration::from_secs(600),
                    0,
                    Some(expiry),
                    has_addr,
                )
                .await;
        }
        match renewed {
            Some(m) => {
                let new = Lease::from_message(&m, iaid).expect("checked by the accept closure");
                if new != lease {
                    let _ = events.send(Event::Bound(new.clone())).await;
                }
                lease = new;
            }
            None => {
                let _ = events.send(Event::Lost).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            ifname: "lan".into(),
            mac: MacAddr([0x00, 0x0f, 0xd3, 0x01, 0x02, 0x03]),
            user_class: "digium_D60_3_0_0".into(),
        }
    }

    #[test]
    fn solicit_layout() {
        let m = build(SOLICIT, 0x0abc_def1, &cfg(), 0, None);
        let b = m.encode();
        assert_eq!(&b[..4], &[1, 0xbc, 0xde, 0xf1]);
        // Client ID: DUID-LL.
        assert_eq!(
            &b[4..18],
            &[0, 1, 0, 10, 0, 3, 0, 1, 0x00, 0x0f, 0xd3, 0x01, 0x02, 0x03]
        );
        let back = Message::parse(&b).unwrap();
        assert_eq!(back, m);
        let ia = back.opt(OPT_IA_NA).unwrap();
        assert_eq!(&ia[..4], &[0xd3, 0x01, 0x02, 0x03]);
        assert!(back.opt(OPT_SERVERID).is_none());
        let info = build(INFORMATION_REQUEST, 1, &cfg(), 0, None);
        assert!(info.opt(OPT_IA_NA).is_none());
    }

    fn reply_with_address(status: Option<u16>) -> Message {
        let mut iaaddr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x1234)
            .octets()
            .to_vec();
        iaaddr.extend_from_slice(&3600u32.to_be_bytes());
        iaaddr.extend_from_slice(&7200u32.to_be_bytes());
        let mut ia = iaid(cfg().mac).to_be_bytes().to_vec();
        ia.extend_from_slice(&0u32.to_be_bytes());
        ia.extend_from_slice(&0u32.to_be_bytes());
        put_opt(&mut ia, OPT_IAADDR, &iaaddr);
        if let Some(s) = status {
            put_opt(&mut ia, OPT_STATUS, &s.to_be_bytes());
        }
        let mut dns = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x53)
            .octets()
            .to_vec();
        dns.extend_from_slice(&Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x54).octets());
        Message {
            msg_type: REPLY,
            xid: 7,
            options: vec![
                (OPT_CLIENTID, duid(cfg().mac)),
                (
                    OPT_SERVERID,
                    vec![0, 1, 0, 1, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
                ),
                (OPT_IA_NA, ia),
                (OPT_DNS, dns),
                (
                    OPT_DOMAIN_LIST,
                    b"\x07example\x03com\x00\x03lan\x00".to_vec(),
                ),
            ],
        }
    }

    #[test]
    fn lease_extraction() {
        let m = Message::parse(&reply_with_address(None).encode()).unwrap();
        let l = Lease::from_message(&m, iaid(cfg().mac)).unwrap();
        assert_eq!(
            l.addresses,
            vec![Address {
                addr: "2001:db8::1234".parse().unwrap(),
                preferred_secs: 3600,
                valid_secs: 7200
            }]
        );
        assert_eq!((l.t1_secs, l.t2_secs), (1800, 2880));
        assert_eq!(l.dns.len(), 2);
        assert_eq!(l.domains, ["example.com", "lan"]);
        // A different IAID is not ours.
        assert!(Lease::from_message(&m, 1).unwrap().addresses.is_empty());
        // NoAddrsAvail (2) inside the IA leaves no addresses.
        let m = reply_with_address(Some(2));
        assert!(
            Lease::from_message(&m, iaid(cfg().mac))
                .unwrap()
                .addresses
                .is_empty()
        );
    }

    #[test]
    fn renew_carries_server_and_address() {
        let l = Lease::from_message(&reply_with_address(None), iaid(cfg().mac)).unwrap();
        let m = build(RENEW, 9, &cfg(), 100, Some(&l));
        assert_eq!(m.opt(OPT_SERVERID), Some(&l.server_id[..]));
        let ia = m.opt(OPT_IA_NA).unwrap();
        let sub = parse_opts(&ia[12..]);
        assert_eq!(sub[0].0, OPT_IAADDR);
        assert_eq!(&sub[0].1[..16], &l.addresses[0].addr.octets());
        assert!(
            build(REBIND, 9, &cfg(), 0, Some(&l))
                .opt(OPT_SERVERID)
                .is_none()
        );
    }

    #[test]
    fn truncated_input_is_rejected() {
        assert!(Message::parse(&[7, 0, 0]).is_none());
        let m = Message::parse(&[7, 0, 0, 1, 0, 3, 0, 40, 1, 2]).unwrap();
        assert!(m.options.is_empty());
    }
}
