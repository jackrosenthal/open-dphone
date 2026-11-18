//! SNTP client (RFC 4330).  Integer arithmetic only: the phone's CPU has no
//! floating-point unit in use.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::net::UdpSocket;
use tokio::time::timeout;

use crate::dns::Resolver;
use crate::sys;

/// Seconds between the NTP epoch (1900) and the Unix epoch (1970).
const NTP_UNIX_OFFSET: i64 = 2_208_988_800;
const NANOS: i128 = 1_000_000_000;

/// Server used when neither DHCP option 42 nor `ntp_server` gives one, as stock.
pub const DEFAULT_SERVER: &str = "pool.ntp.org";
/// Stock bounds one attempt at ten seconds.
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);
/// Stock retries a failed attempt after ten seconds.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(10);
/// Stock default for `ntp_resync`.
pub const DEFAULT_RESYNC: Duration = Duration::from_secs(86_400);

/// One measurement against a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Server clock minus local clock, in nanoseconds.
    pub offset_ns: i128,
    /// Round-trip delay in nanoseconds.
    pub delay_ns: i128,
    pub stratum: u8,
}

/// Unix time in nanoseconds to a 64-bit NTP timestamp.
fn to_ntp(unix_ns: i128) -> u64 {
    let secs = unix_ns.div_euclid(NANOS) as i64 + NTP_UNIX_OFFSET;
    let frac = (unix_ns.rem_euclid(NANOS) << 32) / NANOS;
    ((secs as u64) << 32) | frac as u64
}

/// A 64-bit NTP timestamp to Unix time in nanoseconds.  Timestamps with the
/// top bit clear belong to the era that starts in 2036 (RFC 4330 section 3).
fn from_ntp(ts: u64) -> i128 {
    let mut secs = (ts >> 32) as i64;
    if secs & 0x8000_0000 == 0 {
        secs += 1 << 32;
    }
    let frac = ((ts & 0xffff_ffff) as i128 * NANOS) >> 32;
    (secs - NTP_UNIX_OFFSET) as i128 * NANOS + frac
}

fn now_ns() -> i128 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    }
}

/// Build a client request carrying `transmit` as its transmit timestamp.
pub fn build_request(transmit: u64) -> [u8; 48] {
    let mut p = [0u8; 48];
    p[0] = 0x23; // LI 0, version 4, mode 3 (client)
    p[40..48].copy_from_slice(&transmit.to_be_bytes());
    p
}

/// Check a reply against the request's transmit timestamp `t1` and compute
/// the offset, given the local receive time `t4_ns`.
pub fn parse_reply(p: &[u8], t1: u64, t4_ns: i128) -> io::Result<Sample> {
    let bad = |m: &'static str| io::Error::new(io::ErrorKind::InvalidData, m);
    if p.len() < 48 {
        return Err(bad("short SNTP reply"));
    }
    let mode = p[0] & 7;
    let leap = p[0] >> 6;
    let stratum = p[1];
    if mode != 4 && mode != 5 {
        return Err(bad("SNTP reply is not from a server"));
    }
    if stratum == 0 {
        return Err(bad("SNTP kiss-of-death"));
    }
    if leap == 3 {
        return Err(bad("SNTP server is not synchronized"));
    }
    let u64_at = |o: usize| u64::from_be_bytes(p[o..o + 8].try_into().unwrap());
    if u64_at(24) != t1 {
        return Err(bad("SNTP originate timestamp mismatch"));
    }
    let (t2, t3) = (u64_at(32), u64_at(40));
    if t3 == 0 {
        return Err(bad("SNTP transmit timestamp is zero"));
    }
    let (t1_ns, t2_ns, t3_ns) = (from_ntp(t1), from_ntp(t2), from_ntp(t3));
    Ok(Sample {
        offset_ns: ((t2_ns - t1_ns) + (t3_ns - t4_ns)) / 2,
        delay_ns: (t4_ns - t1_ns) - (t3_ns - t2_ns),
        stratum,
    })
}

/// Ask one server for the time.
pub async fn query(server: SocketAddr) -> io::Result<Sample> {
    let bind: SocketAddr = if server.is_ipv4() {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    } else {
        (Ipv6Addr::UNSPECIFIED, 0).into()
    };
    let sock = UdpSocket::bind(bind).await?;
    sock.connect(server).await?;
    let t1 = to_ntp(now_ns());
    sock.send(&build_request(t1)).await?;
    let mut buf = [0u8; 128];
    loop {
        let n = sock.recv(&mut buf).await?;
        let t4 = now_ns();
        match parse_reply(&buf[..n], t1, t4) {
            Ok(s) => return Ok(s),
            Err(e) => log::debug!("sntp: ignoring reply from {server}: {e}"),
        }
    }
}

/// Step the wall clock by `offset_ns`.
pub fn step_clock(offset_ns: i128) -> io::Result<()> {
    let t = now_ns() + offset_ns;
    sys::set_realtime_clock(t.div_euclid(NANOS) as i64, t.rem_euclid(NANOS) as u32)
}

/// Which servers to try, in order: the first DHCP option-42 server when
/// preferred, else the configured server, else the default pool.
pub fn server_list(
    prefer_option42: bool,
    option42: &[IpAddr],
    configured: Option<&str>,
) -> Vec<String> {
    if prefer_option42 {
        if let Some(ip) = option42.first() {
            return vec![ip.to_string()];
        }
    }
    match configured.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => vec![s.to_string()],
        None => vec![DEFAULT_SERVER.to_string()],
    }
}

/// Resolve `host` and query it, bounded by [`ATTEMPT_TIMEOUT`].  On success
/// the clock is stepped unless `set_clock` is false.
pub async fn sync_once(resolver: &Resolver, host: &str, set_clock: bool) -> io::Result<Sample> {
    let attempt = async {
        let (host, port) = match host.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') && p.parse::<u16>().is_ok() => (h, p.parse().unwrap()),
            _ => (host, 123),
        };
        let addrs = resolver
            .lookup_ip(host, false, false)
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no address for the time server");
        for ip in addrs.into_iter().take(3) {
            match timeout(Duration::from_secs(3), query(SocketAddr::new(ip, port))).await {
                Ok(Ok(s)) => return Ok(s),
                Ok(Err(e)) => last = e,
                Err(_) => last = io::ErrorKind::TimedOut.into(),
            }
        }
        Err(last)
    };
    let sample = timeout(ATTEMPT_TIMEOUT, attempt)
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
    if set_clock {
        step_clock(sample.offset_ns)?;
    }
    Ok(sample)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_conversion() {
        // 2026-11-13T00:00:00Z is Unix 1794528000.
        let unix_ns = 1_794_528_000i128 * NANOS + 500_000_000;
        let ntp = to_ntp(unix_ns);
        assert_eq!(ntp >> 32, 1_794_528_000 + 2_208_988_800);
        assert_eq!(ntp & 0xffff_ffff, 0x8000_0000);
        assert_eq!(from_ntp(ntp), unix_ns);
        // Era 1: 2036-02-07T06:28:16Z is NTP second 2^32, sent as zero.
        assert_eq!(
            from_ntp(5u64 << 32) / NANOS,
            (1i128 << 32) + 5 - NTP_UNIX_OFFSET as i128
        );
    }

    #[test]
    fn offset_and_delay() {
        // Local clock is 100 s behind; 40 ms each way; 10 ms server processing.
        let t1_ns = 1_794_528_000i128 * NANOS;
        let t1 = to_ntp(t1_ns);
        let t2 = to_ntp(t1_ns + 100 * NANOS + 40_000_000);
        let t3 = to_ntp(t1_ns + 100 * NANOS + 50_000_000);
        let t4_ns = t1_ns + 90_000_000;
        let mut reply = [0u8; 48];
        reply[0] = 0x24;
        reply[1] = 2;
        reply[24..32].copy_from_slice(&t1.to_be_bytes());
        reply[32..40].copy_from_slice(&t2.to_be_bytes());
        reply[40..48].copy_from_slice(&t3.to_be_bytes());
        let s = parse_reply(&reply, t1, t4_ns).unwrap();
        assert!((s.offset_ns - 100 * NANOS).abs() < 10, "{}", s.offset_ns);
        assert!((s.delay_ns - 80_000_000).abs() < 10, "{}", s.delay_ns);
        assert_eq!(s.stratum, 2);

        let mut kod = reply;
        kod[1] = 0;
        assert!(parse_reply(&kod, t1, t4_ns).is_err());
        assert!(parse_reply(&reply, t1 ^ 1, t4_ns).is_err());
        let mut unsync = reply;
        unsync[0] = 0xe4;
        assert!(parse_reply(&unsync, t1, t4_ns).is_err());
    }

    #[test]
    fn server_order() {
        let o42 = [
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
        ];
        assert_eq!(server_list(true, &o42, Some("ntp.example")), ["10.0.0.1"]);
        assert_eq!(
            server_list(false, &o42, Some("ntp.example")),
            ["ntp.example"]
        );
        assert_eq!(server_list(true, &[], Some(" ")), [DEFAULT_SERVER]);
    }

    #[tokio::test]
    async fn query_against_fake_server() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let (n, from) = server.recv_from(&mut buf).await.unwrap();
            assert_eq!(n, 48);
            let mut r = [0u8; 48];
            r[0] = 0x24;
            r[1] = 1;
            r[24..32].copy_from_slice(&buf[40..48]);
            let t = to_ntp(now_ns() + 3600 * NANOS);
            r[32..40].copy_from_slice(&t.to_be_bytes());
            r[40..48].copy_from_slice(&t.to_be_bytes());
            server.send_to(&r, from).await.unwrap();
        });
        let s = sync_once(&Resolver::new(), &addr.to_string(), false)
            .await
            .unwrap();
        assert!(
            (s.offset_ns - 3600 * NANOS).abs() < NANOS,
            "{}",
            s.offset_ns
        );
    }
}
