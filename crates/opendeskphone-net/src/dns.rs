//! DNS message codec and a stub resolver.
//!
//! The resolver takes its servers from the network manager (DHCP or static
//! configuration); there is no `resolv.conf`.  It answers the record types
//! SIP needs (RFC 3263): A, AAAA, SRV, and NAPTR.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::{Instant, timeout};

pub const TYPE_A: u16 = 1;
pub const TYPE_CNAME: u16 = 5;
pub const TYPE_PTR: u16 = 12;
pub const TYPE_TXT: u16 = 16;
pub const TYPE_AAAA: u16 = 28;
pub const TYPE_SRV: u16 = 33;
pub const TYPE_NAPTR: u16 = 35;
pub const TYPE_ANY: u16 = 255;
pub const CLASS_IN: u16 = 1;

const FLAG_QR: u16 = 0x8000;
const FLAG_TC: u16 = 0x0200;
const FLAG_RD: u16 = 0x0100;

/// An SRV record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Srv {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

/// A NAPTR record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Naptr {
    pub order: u16,
    pub preference: u16,
    pub flags: String,
    pub service: String,
    pub regexp: String,
    pub replacement: String,
}

/// Decoded record data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RData {
    A(Ipv4Addr),
    Aaaa(Ipv6Addr),
    Cname(String),
    Ptr(String),
    Txt(Vec<Vec<u8>>),
    Srv(Srv),
    Naptr(Naptr),
    Other(Vec<u8>),
}

/// A resource record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    pub rtype: u16,
    /// Class, with the mDNS cache-flush bit removed.
    pub class: u16,
    pub ttl: u32,
    pub data: RData,
}

/// A question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
}

/// A DNS message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub id: u16,
    pub flags: u16,
    pub questions: Vec<Question>,
    pub answers: Vec<Record>,
    pub authority: Vec<Record>,
    pub additional: Vec<Record>,
}

/// Lowercase a name and drop one trailing dot, for comparison.
pub fn normalize(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

fn put_name(out: &mut Vec<u8>, name: &str) -> io::Result<()> {
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() {
            continue;
        }
        if label.len() > 63 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "DNS label too long",
            ));
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    Ok(())
}

/// Read a possibly compressed name at `*pos`, advancing `*pos` past it.
fn get_name(msg: &[u8], pos: &mut usize) -> Option<String> {
    let mut name = String::new();
    let mut p = *pos;
    let mut jumped = false;
    let mut hops = 0;
    loop {
        let len = *msg.get(p)? as usize;
        if len & 0xc0 == 0xc0 {
            let lo = *msg.get(p + 1)? as usize;
            if !jumped {
                *pos = p + 2;
            }
            jumped = true;
            hops += 1;
            if hops > 32 {
                return None;
            }
            p = (len & 0x3f) << 8 | lo;
            continue;
        }
        if len & 0xc0 != 0 {
            return None;
        }
        p += 1;
        if len == 0 {
            break;
        }
        let label = msg.get(p..p + len)?;
        if !name.is_empty() {
            name.push('.');
        }
        name.push_str(&String::from_utf8_lossy(label));
        if name.len() > 255 {
            return None;
        }
        p += len;
    }
    if !jumped {
        *pos = p;
    }
    Some(name)
}

fn get_u16(msg: &[u8], pos: &mut usize) -> Option<u16> {
    let b = msg.get(*pos..*pos + 2)?;
    *pos += 2;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn get_u32(msg: &[u8], pos: &mut usize) -> Option<u32> {
    let b = msg.get(*pos..*pos + 4)?;
    *pos += 4;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn get_string(msg: &[u8], pos: &mut usize, end: usize) -> Option<String> {
    let len = *msg.get(*pos)? as usize;
    let s = msg.get(*pos + 1..*pos + 1 + len)?;
    if *pos + 1 + len > end {
        return None;
    }
    *pos += 1 + len;
    Some(String::from_utf8_lossy(s).into_owned())
}

fn get_record(msg: &[u8], pos: &mut usize) -> Option<Record> {
    let name = get_name(msg, pos)?;
    let rtype = get_u16(msg, pos)?;
    let class = get_u16(msg, pos)? & 0x7fff;
    let ttl = get_u32(msg, pos)?;
    let rdlen = get_u16(msg, pos)? as usize;
    let start = *pos;
    let end = start.checked_add(rdlen)?;
    let raw = msg.get(start..end)?;
    let mut p = start;
    let data = match rtype {
        TYPE_A if rdlen == 4 => RData::A(Ipv4Addr::new(raw[0], raw[1], raw[2], raw[3])),
        TYPE_AAAA if rdlen == 16 => {
            let mut o = [0u8; 16];
            o.copy_from_slice(raw);
            RData::Aaaa(Ipv6Addr::from(o))
        }
        TYPE_CNAME => RData::Cname(get_name(msg, &mut p)?),
        TYPE_PTR => RData::Ptr(get_name(msg, &mut p)?),
        TYPE_TXT => {
            let mut parts = Vec::new();
            let mut i = 0;
            while i < raw.len() {
                let l = raw[i] as usize;
                let part = raw.get(i + 1..i + 1 + l)?;
                parts.push(part.to_vec());
                i += 1 + l;
            }
            RData::Txt(parts)
        }
        TYPE_SRV => {
            let priority = get_u16(msg, &mut p)?;
            let weight = get_u16(msg, &mut p)?;
            let port = get_u16(msg, &mut p)?;
            RData::Srv(Srv {
                priority,
                weight,
                port,
                target: get_name(msg, &mut p)?,
            })
        }
        TYPE_NAPTR => {
            let order = get_u16(msg, &mut p)?;
            let preference = get_u16(msg, &mut p)?;
            let flags = get_string(msg, &mut p, end)?;
            let service = get_string(msg, &mut p, end)?;
            let regexp = get_string(msg, &mut p, end)?;
            RData::Naptr(Naptr {
                order,
                preference,
                flags,
                service,
                regexp,
                replacement: get_name(msg, &mut p)?,
            })
        }
        _ => RData::Other(raw.to_vec()),
    };
    *pos = end;
    Some(Record {
        name,
        rtype,
        class,
        ttl,
        data,
    })
}

impl Message {
    /// A recursive query for one name and type.
    pub fn query(id: u16, name: &str, qtype: u16) -> Self {
        Self {
            id,
            flags: FLAG_RD,
            questions: vec![Question {
                name: name.to_string(),
                qtype,
                qclass: CLASS_IN,
            }],
            ..Default::default()
        }
    }

    pub fn is_response(&self) -> bool {
        self.flags & FLAG_QR != 0
    }

    pub fn truncated(&self) -> bool {
        self.flags & FLAG_TC != 0
    }

    pub fn rcode(&self) -> u8 {
        (self.flags & 0xf) as u8
    }

    pub fn parse(msg: &[u8]) -> Option<Self> {
        let mut pos = 0;
        let id = get_u16(msg, &mut pos)?;
        let flags = get_u16(msg, &mut pos)?;
        let counts = [
            get_u16(msg, &mut pos)?,
            get_u16(msg, &mut pos)?,
            get_u16(msg, &mut pos)?,
            get_u16(msg, &mut pos)?,
        ];
        let mut m = Message {
            id,
            flags,
            ..Default::default()
        };
        for _ in 0..counts[0] {
            let name = get_name(msg, &mut pos)?;
            let qtype = get_u16(msg, &mut pos)?;
            let qclass = get_u16(msg, &mut pos)?;
            m.questions.push(Question {
                name,
                qtype,
                qclass,
            });
        }
        'sections: for i in 0..3 {
            for _ in 0..counts[i + 1] {
                match get_record(msg, &mut pos) {
                    Some(r) => match i {
                        0 => m.answers.push(r),
                        1 => m.authority.push(r),
                        _ => m.additional.push(r),
                    },
                    // A truncated message may end inside a record; keep what parsed.
                    None if flags & FLAG_TC != 0 => break 'sections,
                    None => return None,
                }
            }
        }
        Some(m)
    }

    /// Encode without name compression.  Only the record types this crate
    /// originates (A, AAAA, PTR, SRV, TXT) are supported in the record
    /// sections; others are written from `RData::Other`.
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        let mut b = Vec::with_capacity(64);
        b.extend_from_slice(&self.id.to_be_bytes());
        b.extend_from_slice(&self.flags.to_be_bytes());
        for n in [
            self.questions.len(),
            self.answers.len(),
            self.authority.len(),
            self.additional.len(),
        ] {
            b.extend_from_slice(&(n as u16).to_be_bytes());
        }
        for q in &self.questions {
            put_name(&mut b, &q.name)?;
            b.extend_from_slice(&q.qtype.to_be_bytes());
            b.extend_from_slice(&q.qclass.to_be_bytes());
        }
        for r in self
            .answers
            .iter()
            .chain(&self.authority)
            .chain(&self.additional)
        {
            put_name(&mut b, &r.name)?;
            b.extend_from_slice(&r.rtype.to_be_bytes());
            b.extend_from_slice(&r.class.to_be_bytes());
            b.extend_from_slice(&r.ttl.to_be_bytes());
            let mut rd = Vec::new();
            match &r.data {
                RData::A(a) => rd.extend_from_slice(&a.octets()),
                RData::Aaaa(a) => rd.extend_from_slice(&a.octets()),
                RData::Cname(n) | RData::Ptr(n) => put_name(&mut rd, n)?,
                RData::Txt(parts) => {
                    for p in parts {
                        rd.push(p.len().min(255) as u8);
                        rd.extend_from_slice(&p[..p.len().min(255)]);
                    }
                }
                RData::Srv(s) => {
                    rd.extend_from_slice(&s.priority.to_be_bytes());
                    rd.extend_from_slice(&s.weight.to_be_bytes());
                    rd.extend_from_slice(&s.port.to_be_bytes());
                    put_name(&mut rd, &s.target)?;
                }
                RData::Naptr(n) => {
                    rd.extend_from_slice(&n.order.to_be_bytes());
                    rd.extend_from_slice(&n.preference.to_be_bytes());
                    for s in [&n.flags, &n.service, &n.regexp] {
                        rd.push(s.len().min(255) as u8);
                        rd.extend_from_slice(&s.as_bytes()[..s.len().min(255)]);
                    }
                    put_name(&mut rd, &n.replacement)?;
                }
                RData::Other(raw) => rd.extend_from_slice(raw),
            }
            b.extend_from_slice(&(rd.len() as u16).to_be_bytes());
            b.extend_from_slice(&rd);
        }
        Ok(b)
    }
}

/// Why a lookup failed.
#[derive(Debug)]
pub enum Error {
    /// No servers are configured.
    NoServers,
    /// The name does not exist (NXDOMAIN).
    NoSuchName,
    /// The name exists but has no record of the requested type.
    NoData,
    /// Every server failed or timed out.
    Timeout,
    /// A server answered with an error code.
    Server(u8),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoServers => write!(f, "no DNS servers configured"),
            Self::NoSuchName => write!(f, "no such name"),
            Self::NoData => write!(f, "no records of that type"),
            Self::Timeout => write!(f, "DNS servers did not answer"),
            Self::Server(c) => write!(f, "DNS server error {c}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

struct CacheEntry {
    expires: Instant,
    records: Vec<Record>,
}

#[derive(Default)]
struct Inner {
    servers: Vec<SocketAddr>,
    search: Option<String>,
    cache: HashMap<(String, u16), CacheEntry>,
    next_id: u16,
}

/// The stub resolver.  Cheap to clone; clones share servers and cache.
#[derive(Clone, Default)]
pub struct Resolver {
    inner: Arc<Mutex<Inner>>,
}

const CACHE_MAX: usize = 64;
const MAX_TTL: u32 = 3600;
const UDP_TIMEOUT: Duration = Duration::from_secs(2);
const ROUNDS: usize = 2;

impl Resolver {
    pub fn new() -> Self {
        let r = Self::default();
        r.inner.lock().unwrap().next_id = std::process::id() as u16 ^ 0x5a5a;
        r
    }

    /// Replace the server list (port 53) and drop the cache.
    pub fn set_servers(&self, servers: &[IpAddr]) {
        let mut g = self.inner.lock().unwrap();
        let new: Vec<SocketAddr> = servers.iter().map(|&ip| SocketAddr::new(ip, 53)).collect();
        if g.servers != new {
            g.servers = new;
            g.cache.clear();
        }
    }

    /// Replace the server list with explicit ports (used by tests).
    pub fn set_server_addrs(&self, servers: &[SocketAddr]) {
        let mut g = self.inner.lock().unwrap();
        g.servers = servers.to_vec();
        g.cache.clear();
    }

    /// Domain appended to names without a dot.
    pub fn set_search_domain(&self, domain: Option<&str>) {
        self.inner.lock().unwrap().search = domain.map(normalize).filter(|d| !d.is_empty());
    }

    pub fn servers(&self) -> Vec<SocketAddr> {
        self.inner.lock().unwrap().servers.clone()
    }

    fn cached(&self, name: &str, qtype: u16) -> Option<Vec<Record>> {
        let mut g = self.inner.lock().unwrap();
        let key = (normalize(name), qtype);
        match g.cache.get(&key) {
            Some(e) if e.expires > Instant::now() => Some(e.records.clone()),
            Some(_) => {
                g.cache.remove(&key);
                None
            }
            None => None,
        }
    }

    fn store(&self, name: &str, qtype: u16, records: &[Record]) {
        let ttl = records
            .iter()
            .map(|r| r.ttl)
            .min()
            .unwrap_or(0)
            .min(MAX_TTL);
        if ttl == 0 {
            return;
        }
        let mut g = self.inner.lock().unwrap();
        if g.cache.len() >= CACHE_MAX {
            let now = Instant::now();
            g.cache.retain(|_, e| e.expires > now);
            if g.cache.len() >= CACHE_MAX {
                g.cache.clear();
            }
        }
        g.cache.insert(
            (normalize(name), qtype),
            CacheEntry {
                expires: Instant::now() + Duration::from_secs(ttl as u64),
                records: records.to_vec(),
            },
        );
    }

    /// Ask the configured servers one question.  Returns the answer section
    /// (which may include CNAMEs) and the additional section.
    pub async fn query(&self, name: &str, qtype: u16) -> Result<(Vec<Record>, Vec<Record>), Error> {
        let (servers, id) = {
            let mut g = self.inner.lock().unwrap();
            g.next_id = g.next_id.wrapping_mul(25173).wrapping_add(13849);
            (g.servers.clone(), g.next_id)
        };
        if servers.is_empty() {
            return Err(Error::NoServers);
        }
        let q = Message::query(id, name, qtype).encode()?;
        let mut last = Error::Timeout;
        for _ in 0..ROUNDS {
            for server in &servers {
                match exchange(*server, &q, id).await {
                    Ok(m) => match m.rcode() {
                        0 => return Ok((m.answers, m.additional)),
                        3 => return Err(Error::NoSuchName),
                        c => last = Error::Server(c),
                    },
                    Err(e) => {
                        log::debug!("dns: {server} failed for {name}: {e}");
                        if !matches!(last, Error::Server(_)) {
                            last = Error::Timeout;
                        }
                    }
                }
            }
        }
        Err(last)
    }

    /// Records of one type for a name, following CNAMEs within the answer.
    pub async fn lookup(&self, name: &str, qtype: u16) -> Result<Vec<Record>, Error> {
        if let Some(r) = self.cached(name, qtype) {
            return Ok(r);
        }
        let (answers, _) = self.query(name, qtype).await?;
        let mut target = normalize(name);
        for _ in 0..8 {
            let next = answers.iter().find_map(|r| match &r.data {
                RData::Cname(c) if normalize(&r.name) == target && qtype != TYPE_CNAME => {
                    Some(normalize(c))
                }
                _ => None,
            });
            match next {
                Some(n) => target = n,
                None => break,
            }
        }
        let records: Vec<Record> = answers
            .into_iter()
            .filter(|r| r.rtype == qtype && normalize(&r.name) == target)
            .collect();
        if records.is_empty() {
            return Err(Error::NoData);
        }
        self.store(name, qtype, &records);
        Ok(records)
    }

    fn candidates(&self, host: &str) -> Vec<String> {
        let host = host.trim();
        let search = self.inner.lock().unwrap().search.clone();
        match search {
            Some(d) if !host.contains('.') => vec![format!("{host}.{d}"), host.to_string()],
            _ => vec![host.to_string()],
        }
    }

    /// IPv4 addresses for a host name or literal.
    pub async fn lookup_ipv4(&self, host: &str) -> Result<Vec<Ipv4Addr>, Error> {
        if let Ok(ip) = host.trim().parse::<Ipv4Addr>() {
            return Ok(vec![ip]);
        }
        let mut last = Error::NoData;
        for name in self.candidates(host) {
            match self.lookup(&name, TYPE_A).await {
                Ok(r) => {
                    return Ok(r
                        .iter()
                        .filter_map(|r| {
                            if let RData::A(a) = r.data {
                                Some(a)
                            } else {
                                None
                            }
                        })
                        .collect());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// IPv6 addresses for a host name or literal.
    pub async fn lookup_ipv6(&self, host: &str) -> Result<Vec<Ipv6Addr>, Error> {
        if let Ok(ip) = host
            .trim()
            .trim_matches(|c| c == '[' || c == ']')
            .parse::<Ipv6Addr>()
        {
            return Ok(vec![ip]);
        }
        let mut last = Error::NoData;
        for name in self.candidates(host) {
            match self.lookup(&name, TYPE_AAAA).await {
                Ok(r) => {
                    return Ok(r
                        .iter()
                        .filter_map(|r| {
                            if let RData::Aaaa(a) = r.data {
                                Some(a)
                            } else {
                                None
                            }
                        })
                        .collect());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Addresses for a host name or literal.  `prefer_v6` orders the
    /// result and decides which family is asked first; the other family is
    /// asked only when `want_both` is set or the first has nothing.
    pub async fn lookup_ip(
        &self,
        host: &str,
        prefer_v6: bool,
        want_both: bool,
    ) -> Result<Vec<IpAddr>, Error> {
        if let Ok(ip) = host
            .trim()
            .trim_matches(|c| c == '[' || c == ']')
            .parse::<IpAddr>()
        {
            return Ok(vec![ip]);
        }
        let mut out: Vec<IpAddr> = Vec::new();
        let mut err = None;
        for v6 in if prefer_v6 {
            [true, false]
        } else {
            [false, true]
        } {
            let r = if v6 {
                self.lookup_ipv6(host)
                    .await
                    .map(|v| v.into_iter().map(IpAddr::V6).collect::<Vec<_>>())
            } else {
                self.lookup_ipv4(host)
                    .await
                    .map(|v| v.into_iter().map(IpAddr::V4).collect::<Vec<_>>())
            };
            match r {
                Ok(v) => out.extend(v),
                Err(e) => err = err.or(Some(e)),
            }
            if !out.is_empty() && !want_both {
                break;
            }
        }
        if out.is_empty() {
            Err(err.unwrap_or(Error::NoData))
        } else {
            Ok(out)
        }
    }

    /// SRV records, sorted by priority and then by descending weight.
    pub async fn lookup_srv(&self, name: &str) -> Result<Vec<Srv>, Error> {
        let mut v: Vec<Srv> = self
            .lookup(name, TYPE_SRV)
            .await?
            .into_iter()
            .filter_map(|r| {
                if let RData::Srv(s) = r.data {
                    Some(s)
                } else {
                    None
                }
            })
            .collect();
        // A single record with target "." means the service is not offered (RFC 2782).
        if v.len() == 1 && normalize(&v[0].target).is_empty() {
            return Err(Error::NoData);
        }
        v.sort_by(|a, b| a.priority.cmp(&b.priority).then(b.weight.cmp(&a.weight)));
        Ok(v)
    }

    /// NAPTR records, sorted by order and preference.
    pub async fn lookup_naptr(&self, name: &str) -> Result<Vec<Naptr>, Error> {
        let mut v: Vec<Naptr> = self
            .lookup(name, TYPE_NAPTR)
            .await?
            .into_iter()
            .filter_map(|r| {
                if let RData::Naptr(n) = r.data {
                    Some(n)
                } else {
                    None
                }
            })
            .collect();
        v.sort_by(|a, b| a.order.cmp(&b.order).then(a.preference.cmp(&b.preference)));
        Ok(v)
    }
}

async fn exchange(server: SocketAddr, query: &[u8], id: u16) -> io::Result<Message> {
    let bind: SocketAddr = if server.is_ipv4() {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    } else {
        (Ipv6Addr::UNSPECIFIED, 0).into()
    };
    let sock = UdpSocket::bind(bind).await?;
    sock.connect(server).await?;
    sock.send(query).await?;
    let mut buf = [0u8; 1500];
    let deadline = Instant::now() + UDP_TIMEOUT;
    loop {
        let n = tokio::time::timeout_at(deadline, sock.recv(&mut buf))
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        let Some(m) = Message::parse(&buf[..n]) else {
            continue;
        };
        if m.id != id || !m.is_response() {
            continue;
        }
        if m.truncated() {
            return exchange_tcp(server, query, id).await;
        }
        return Ok(m);
    }
}

async fn exchange_tcp(server: SocketAddr, query: &[u8], id: u16) -> io::Result<Message> {
    let fut = async {
        let mut s = TcpStream::connect(server).await?;
        let mut framed = (query.len() as u16).to_be_bytes().to_vec();
        framed.extend_from_slice(query);
        s.write_all(&framed).await?;
        let mut len = [0u8; 2];
        s.read_exact(&mut len).await?;
        let mut buf = vec![0u8; u16::from_be_bytes(len) as usize];
        s.read_exact(&mut buf).await?;
        let m = Message::parse(&buf)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad DNS reply"))?;
        if m.id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "DNS reply ID mismatch",
            ));
        }
        Ok(m)
    };
    timeout(Duration::from_secs(5), fut)
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The response from RFC 1035 section 4.1.4's compression discussion is
    /// prose only, so this is a hand-assembled reply using compression
    /// pointers the way real servers do.
    fn compressed_reply() -> Vec<u8> {
        let mut b = vec![0x12, 0x34, 0x81, 0x80, 0, 1, 0, 2, 0, 0, 0, 1];
        // Question: _sip._udp.example.com SRV IN, at offset 12.
        b.extend_from_slice(b"\x04_sip\x04_udp\x07example\x03com\x00");
        b.extend_from_slice(&[0, 33, 0, 1]);
        // Answer 1: pointer to offset 12; target "pbx" + pointer to "example.com" (offset 22).
        b.extend_from_slice(&[
            0xc0, 12, 0, 33, 0, 1, 0, 0, 0, 60, 0, 12, 0, 10, 0, 5, 0x13, 0xc4,
        ]);
        b.extend_from_slice(b"\x03pbx\xc0\x16");
        // Answer 2: lower priority.
        b.extend_from_slice(&[
            0xc0, 12, 0, 33, 0, 1, 0, 0, 0, 60, 0, 13, 0, 20, 0, 0, 0x13, 0xc4,
        ]);
        b.extend_from_slice(b"\x04pbx2\xc0\x16");
        // Additional: pbx.example.com A 192.0.2.1; name is a pointer into answer 1's target.
        let target_off = (12 + 23 + 4 + 18) as u8;
        b.extend_from_slice(&[
            0xc0, target_off, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 1,
        ]);
        b
    }

    #[test]
    fn parses_compressed_names() {
        let m = Message::parse(&compressed_reply()).unwrap();
        assert!(m.is_response());
        assert_eq!(m.questions[0].name, "_sip._udp.example.com");
        assert_eq!(m.answers.len(), 2);
        assert_eq!(
            m.answers[0].data,
            RData::Srv(Srv {
                priority: 10,
                weight: 5,
                port: 5060,
                target: "pbx.example.com".into()
            })
        );
        assert_eq!(m.additional[0].name, "pbx.example.com");
        assert_eq!(m.additional[0].data, RData::A(Ipv4Addr::new(192, 0, 2, 1)));
    }

    #[test]
    fn pointer_loops_are_rejected() {
        let mut b = vec![0, 1, 0x81, 0x80, 0, 1, 0, 0, 0, 0, 0, 0];
        b.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1]);
        assert!(Message::parse(&b).is_none());
    }

    #[test]
    fn encode_round_trip() {
        let mut m = Message::query(7, "example.com.", TYPE_NAPTR);
        m.flags |= FLAG_QR;
        m.answers.push(Record {
            name: "example.com".into(),
            rtype: TYPE_NAPTR,
            class: CLASS_IN,
            ttl: 300,
            data: RData::Naptr(Naptr {
                order: 10,
                preference: 20,
                flags: "s".into(),
                service: "SIP+D2U".into(),
                regexp: String::new(),
                replacement: "_sip._udp.example.com".into(),
            }),
        });
        m.additional.push(Record {
            name: "t.example.com".into(),
            rtype: TYPE_TXT,
            class: CLASS_IN,
            ttl: 1,
            data: RData::Txt(vec![b"a=b".to_vec()]),
        });
        let b = m.encode().unwrap();
        let mut back = Message::parse(&b).unwrap();
        back.questions[0].name.push('.');
        assert_eq!(back, m);
    }

    async fn fake_server(replies: Vec<Message>) -> SocketAddr {
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = sock.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                    return;
                };
                let q = Message::parse(&buf[..n]).unwrap();
                let want = &q.questions[0];
                let mut r = Message {
                    id: q.id,
                    flags: FLAG_QR | FLAG_RD | 0x80,
                    questions: q.questions.clone(),
                    ..Default::default()
                };
                let mut found = false;
                for m in &replies {
                    if normalize(&m.questions[0].name) == normalize(&want.name)
                        && m.questions[0].qtype == want.qtype
                    {
                        r.answers = m.answers.clone();
                        found = true;
                    }
                }
                if !found {
                    r.flags |= 3;
                }
                let _ = sock.send_to(&r.encode().unwrap(), from).await;
            }
        });
        addr
    }

    fn rec(name: &str, rtype: u16, data: RData) -> Record {
        Record {
            name: name.into(),
            rtype,
            class: CLASS_IN,
            ttl: 60,
            data,
        }
    }

    #[tokio::test]
    async fn resolver_against_fake_server() {
        let mut a = Message::query(0, "pbx.example.com", TYPE_A);
        a.answers.push(rec(
            "pbx.example.com",
            TYPE_CNAME,
            RData::Cname("real.example.com".into()),
        ));
        a.answers.push(rec(
            "real.example.com",
            TYPE_A,
            RData::A(Ipv4Addr::new(192, 0, 2, 7)),
        ));
        let mut srv = Message::query(0, "_sip._udp.example.com", TYPE_SRV);
        for (prio, weight, t) in [(20, 0, "b"), (10, 1, "a2"), (10, 9, "a1")] {
            srv.answers.push(rec(
                "_sip._udp.example.com",
                TYPE_SRV,
                RData::Srv(Srv {
                    priority: prio,
                    weight,
                    port: 5060,
                    target: format!("{t}.example.com"),
                }),
            ));
        }
        let mut short = Message::query(0, "pbx.lan.test", TYPE_A);
        short.answers.push(rec(
            "pbx.lan.test",
            TYPE_A,
            RData::A(Ipv4Addr::new(10, 0, 0, 9)),
        ));
        let addr = fake_server(vec![a, srv, short]).await;

        let r = Resolver::new();
        assert!(matches!(
            r.lookup_ipv4("pbx.example.com").await,
            Err(Error::NoServers)
        ));
        r.set_server_addrs(&[addr]);
        assert_eq!(
            r.lookup_ipv4("pbx.example.com").await.unwrap(),
            vec![Ipv4Addr::new(192, 0, 2, 7)]
        );
        let s = r.lookup_srv("_sip._udp.example.com").await.unwrap();
        assert_eq!(
            s.iter().map(|s| s.target.as_str()).collect::<Vec<_>>(),
            ["a1.example.com", "a2.example.com", "b.example.com"]
        );
        assert!(matches!(
            r.lookup_ipv4("missing.example.com").await,
            Err(Error::NoSuchName)
        ));
        r.set_search_domain(Some("lan.test"));
        assert_eq!(
            r.lookup_ipv4("pbx").await.unwrap(),
            vec![Ipv4Addr::new(10, 0, 0, 9)]
        );
        assert_eq!(
            r.lookup_ip("192.0.2.200", false, false).await.unwrap(),
            vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 200))]
        );
    }
}
