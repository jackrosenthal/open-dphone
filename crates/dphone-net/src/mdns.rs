//! Multicast DNS service discovery (RFC 6762/6763), query side only.
//!
//! The phone looks for a provisioning server announced as
//! `_digiumproxy._udp.local`.  The stock firmware runs no mDNS responder,
//! so neither does this.

use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::fd::AsRawFd;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::{Instant, timeout_at};

use crate::dns::{
    CLASS_IN, Message, Question, RData, TYPE_A, TYPE_AAAA, TYPE_PTR, TYPE_SRV, TYPE_TXT, normalize,
};
use crate::sys;

/// The DNS-SD type DPMA servers announce.
pub const DIGIUM_PROXY_SERVICE: &str = "_digiumproxy._udp.local";

const MDNS_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const MDNS_PORT: u16 = 5353;
/// Top bit of the question class: a unicast response is acceptable.
const QU_BIT: u16 = 0x8000;

/// One discovered service instance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Service {
    /// Instance name without the service type, e.g., `Digium Phones Config Server`.
    pub instance: String,
    /// Full instance name, e.g., `Name._digiumproxy._udp.local`.
    pub fullname: String,
    pub host: String,
    pub port: u16,
    pub addrs: Vec<IpAddr>,
    /// TXT entries as key/value pairs; a key without `=` has an empty value.
    pub txt: BTreeMap<String, String>,
}

#[derive(Default)]
struct Collected {
    /// Normalized full instance name and the name as announced.
    ptr: Vec<(String, String)>,
    srv: BTreeMap<String, (String, u16)>,
    txt: BTreeMap<String, BTreeMap<String, String>>,
    addrs: BTreeMap<String, Vec<IpAddr>>,
}

impl Collected {
    fn absorb(&mut self, service: &str, m: &Message) {
        for r in m.answers.iter().chain(&m.additional) {
            let name = normalize(&r.name);
            match &r.data {
                RData::Ptr(target) if name == service => {
                    let t = normalize(target);
                    if !self.ptr.iter().any(|(n, _)| *n == t) {
                        self.ptr.push((t, target.trim_end_matches('.').to_string()));
                    }
                }
                RData::Srv(s) => {
                    self.srv.insert(name, (normalize(&s.target), s.port));
                }
                RData::Txt(parts) => {
                    let e = self.txt.entry(name).or_default();
                    for p in parts {
                        let s = String::from_utf8_lossy(p);
                        if s.is_empty() {
                            continue;
                        }
                        match s.split_once('=') {
                            Some((k, v)) => e.insert(k.to_string(), v.to_string()),
                            None => e.insert(s.to_string(), String::new()),
                        };
                    }
                }
                RData::A(a) => {
                    let v = self.addrs.entry(name).or_default();
                    if !v.contains(&IpAddr::V4(*a)) {
                        v.push(IpAddr::V4(*a));
                    }
                }
                RData::Aaaa(a) => {
                    let v = self.addrs.entry(name).or_default();
                    if !v.contains(&IpAddr::V6(*a)) {
                        v.push(IpAddr::V6(*a));
                    }
                }
                _ => {}
            }
        }
    }

    /// Questions still needed to complete every instance.
    fn missing(&self) -> Vec<Question> {
        let mut q = Vec::new();
        for (inst, _) in &self.ptr {
            match self.srv.get(inst) {
                None => {
                    q.push(Question {
                        name: inst.clone(),
                        qtype: TYPE_SRV,
                        qclass: CLASS_IN,
                    });
                    q.push(Question {
                        name: inst.clone(),
                        qtype: TYPE_TXT,
                        qclass: CLASS_IN,
                    });
                }
                Some((host, _)) if !self.addrs.contains_key(host) => {
                    q.push(Question {
                        name: host.clone(),
                        qtype: TYPE_A,
                        qclass: CLASS_IN,
                    });
                    q.push(Question {
                        name: host.clone(),
                        qtype: TYPE_AAAA,
                        qclass: CLASS_IN,
                    });
                }
                Some(_) => {}
            }
        }
        q
    }

    fn services(&self, service: &str) -> Vec<Service> {
        self.ptr
            .iter()
            .filter_map(|(inst, shown)| {
                let (host, port) = self.srv.get(inst)?;
                let cut = shown.len().saturating_sub(service.len() + 1);
                let instance = if inst.ends_with(service) && shown.is_char_boundary(cut) {
                    shown[..cut].to_string()
                } else {
                    shown.clone()
                };
                Some(Service {
                    instance,
                    fullname: inst.clone(),
                    host: host.clone(),
                    port: *port,
                    addrs: self.addrs.get(host).cloned().unwrap_or_default(),
                    txt: self.txt.get(inst).cloned().unwrap_or_default(),
                })
            })
            .collect()
    }
}

/// Build the response-collecting logic's input: used by the tests and by
/// [`browse`].
pub fn services_from_messages(service: &str, messages: &[Message]) -> Vec<Service> {
    let service = normalize(service);
    let mut c = Collected::default();
    for m in messages {
        c.absorb(&service, m);
    }
    c.services(&service)
}

fn open_socket(ifname: &str, local: Ipv4Addr) -> io::Result<UdpSocket> {
    let fd = sys::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)?;
    let raw = fd.as_raw_fd();
    sys::setsockopt(raw, libc::SOL_SOCKET, libc::SO_REUSEADDR, &1i32)?;
    let _ = sys::setsockopt_bytes(
        raw,
        libc::SOL_SOCKET,
        libc::SO_BINDTODEVICE,
        ifname.as_bytes(),
    );
    let sock = std::net::UdpSocket::from(fd);
    // Bind to port 5353 so multicast responses reach us; fall back to an
    // ephemeral port (responders then answer by unicast, which the QU bit
    // asks for anyway).
    let bound = bind_v4(raw, MDNS_PORT).or_else(|_| bind_v4(raw, 0));
    bound?;
    let mreq = libc::ip_mreqn {
        imr_multiaddr: libc::in_addr {
            s_addr: u32::from(MDNS_GROUP).to_be(),
        },
        imr_address: libc::in_addr {
            s_addr: u32::from(local).to_be(),
        },
        imr_ifindex: sys::if_index(ifname).unwrap_or(0) as i32,
    };
    let _ = sys::setsockopt(raw, libc::IPPROTO_IP, libc::IP_ADD_MEMBERSHIP, &mreq);
    sys::setsockopt(raw, libc::IPPROTO_IP, libc::IP_MULTICAST_IF, &mreq)?;
    sys::setsockopt(raw, libc::IPPROTO_IP, libc::IP_MULTICAST_TTL, &255i32)?;
    sys::setsockopt(raw, libc::IPPROTO_IP, libc::IP_MULTICAST_LOOP, &0i32)?;
    UdpSocket::from_std(sock)
}

fn bind_v4(fd: i32, port: u16) -> io::Result<()> {
    let sa = libc::sockaddr_in {
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: port.to_be(),
        sin_addr: libc::in_addr { s_addr: 0 },
        sin_zero: [0; 8],
    };
    // SAFETY: `sa` is a valid `sockaddr_in` of the length passed.
    sys::cvt(unsafe {
        libc::bind(
            fd,
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    })?;
    Ok(())
}

/// Browse for `service` on `ifname` (which has address `local`) for
/// `window`, and return the instances that resolved to a host and port.
pub async fn browse(
    ifname: &str,
    local: Ipv4Addr,
    service: &str,
    window: Duration,
) -> io::Result<Vec<Service>> {
    let service = normalize(service);
    let sock = open_socket(ifname, local)?;
    let dest = SocketAddr::new(IpAddr::V4(MDNS_GROUP), MDNS_PORT);
    let mut collected = Collected::default();
    let end = Instant::now() + window;
    let mut next_send = Instant::now();
    let mut interval = Duration::from_millis(250);
    let mut buf = [0u8; 9000];
    loop {
        let now = Instant::now();
        if now >= end {
            break;
        }
        if now >= next_send {
            let mut q = Message::default();
            q.questions.push(Question {
                name: service.clone(),
                qtype: TYPE_PTR,
                qclass: CLASS_IN | QU_BIT,
            });
            for mut extra in collected.missing() {
                extra.qclass |= QU_BIT;
                q.questions.push(extra);
            }
            if let Err(e) = sock.send_to(&q.encode()?, dest).await {
                log::debug!("mdns: send failed: {e}");
            }
            next_send = now + interval;
            interval = (interval * 2).min(Duration::from_secs(2));
        }
        let had_missing = !collected.missing().is_empty();
        match timeout_at(next_send.min(end), sock.recv_from(&mut buf)).await {
            Ok(Ok((n, _from))) => {
                if let Some(m) = Message::parse(&buf[..n]) {
                    if m.is_response() {
                        collected.absorb(&service, &m);
                        // New instances need follow-up questions right away.
                        if !had_missing && !collected.missing().is_empty() {
                            next_send = Instant::now();
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                log::debug!("mdns: receive failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(_) => {}
        }
    }
    Ok(collected.services(&service))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{Record, Srv};

    fn rec(name: &str, rtype: u16, data: RData) -> Record {
        Record {
            name: name.into(),
            rtype,
            class: CLASS_IN,
            ttl: 120,
            data,
        }
    }

    #[test]
    fn assembles_a_service_from_split_responses() {
        let inst = "Digium Phones Config Server._digiumproxy._udp.local";
        let mut a = Message {
            flags: 0x8400,
            ..Default::default()
        };
        a.answers
            .push(rec(DIGIUM_PROXY_SERVICE, TYPE_PTR, RData::Ptr(inst.into())));
        let mut c = Collected::default();
        c.absorb(DIGIUM_PROXY_SERVICE, &a);
        let missing = c.missing();
        assert_eq!(missing.len(), 2);
        assert_eq!(missing[0].qtype, TYPE_SRV);

        let mut b = Message {
            flags: 0x8400,
            ..Default::default()
        };
        b.answers.push(rec(
            inst,
            TYPE_SRV,
            RData::Srv(Srv {
                priority: 0,
                weight: 0,
                port: 5060,
                target: "PBX.local".into(),
            }),
        ));
        b.answers.push(rec(
            inst,
            TYPE_TXT,
            RData::Txt(vec![
                b"sipUrl=sip:proxy@10.0.0.2:5060".to_vec(),
                b"flag".to_vec(),
            ]),
        ));
        b.additional.push(rec(
            "pbx.local",
            TYPE_A,
            RData::A(Ipv4Addr::new(10, 0, 0, 2)),
        ));
        // The messages survive an encode/parse round trip on the way.
        let b = Message::parse(&b.encode().unwrap()).unwrap();
        let s = services_from_messages(DIGIUM_PROXY_SERVICE, &[a, b]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].instance, "Digium Phones Config Server");
        assert_eq!((s[0].host.as_str(), s[0].port), ("pbx.local", 5060));
        assert_eq!(s[0].addrs, vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))]);
        assert_eq!(s[0].txt["sipUrl"], "sip:proxy@10.0.0.2:5060");
        assert_eq!(s[0].txt["flag"], "");
    }
}
