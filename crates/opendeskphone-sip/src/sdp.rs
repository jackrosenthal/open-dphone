//! SDP for one audio stream: parsing, building, and offer/answer
//! (RFC 3264) with RFC 4733 telephone events and SDES SRTP (RFC 4568).

use base64::Engine;
use opendeskphone_state::Codec;
use std::fmt::Write;
use std::net::{IpAddr, SocketAddr};

/// Payload type we offer for telephone events.
pub const DTMF_PT: u8 = 101;

/// Stream direction from the point of view of whoever wrote the SDP.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

impl Direction {
    pub fn token(self) -> &'static str {
        match self {
            Direction::SendRecv => "sendrecv",
            Direction::SendOnly => "sendonly",
            Direction::RecvOnly => "recvonly",
            Direction::Inactive => "inactive",
        }
    }
    pub fn sends(self) -> bool {
        matches!(self, Direction::SendRecv | Direction::SendOnly)
    }
    pub fn receives(self) -> bool {
        matches!(self, Direction::SendRecv | Direction::RecvOnly)
    }
    /// The direction to answer with, given what we are willing to do.
    pub fn answer(self, ours: Direction) -> Direction {
        let send = ours.sends() && self.receives();
        let recv = ours.receives() && self.sends();
        match (send, recv) {
            (true, true) => Direction::SendRecv,
            (true, false) => Direction::SendOnly,
            (false, true) => Direction::RecvOnly,
            (false, false) => Direction::Inactive,
        }
    }
}

/// SRTP crypto suites of RFC 4568 that the media engine implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SrtpSuite {
    AesCm128HmacSha1_80,
    AesCm128HmacSha1_32,
}

impl SrtpSuite {
    pub fn name(self) -> &'static str {
        match self {
            SrtpSuite::AesCm128HmacSha1_80 => "AES_CM_128_HMAC_SHA1_80",
            SrtpSuite::AesCm128HmacSha1_32 => "AES_CM_128_HMAC_SHA1_32",
        }
    }
    fn from_name(s: &str) -> Option<SrtpSuite> {
        match s {
            "AES_CM_128_HMAC_SHA1_80" => Some(SrtpSuite::AesCm128HmacSha1_80),
            "AES_CM_128_HMAC_SHA1_32" => Some(SrtpSuite::AesCm128HmacSha1_32),
            _ => None,
        }
    }
}

/// One `a=crypto` line: 16 bytes of master key and 14 of master salt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crypto {
    pub tag: u32,
    pub suite: SrtpSuite,
    pub key_salt: [u8; 30],
}

impl Crypto {
    /// A fresh key for `suite`.
    pub fn generate(tag: u32, suite: SrtpSuite) -> Result<Crypto, String> {
        let mut key_salt = [0u8; 30];
        getrandom::fill(&mut key_salt).map_err(|e| format!("no entropy for an SRTP key: {e}"))?;
        Ok(Crypto { tag, suite, key_salt })
    }
}

/// One `a=rtpmap` format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Format {
    pub pt: u8,
    /// Encoding name as written (`PCMU`, `telephone-event`, ...).
    pub name: String,
    pub clock: u32,
    pub fmtp: String,
}

/// A session description with (at most) one audio stream that we use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sdp {
    pub session_id: u64,
    pub version: u64,
    pub addr: IpAddr,
    pub port: u16,
    /// `RTP/SAVP` rather than `RTP/AVP`.
    pub secure: bool,
    pub formats: Vec<Format>,
    pub direction: Direction,
    pub ptime: Option<u16>,
    pub crypto: Vec<Crypto>,
    /// Count of further `m=` lines (video and so on), which we refuse.
    pub other_media: Vec<String>,
}

/// The static payload types we know without an rtpmap.
fn static_format(pt: u8) -> Option<(&'static str, u32)> {
    match pt {
        0 => Some(("PCMU", 8000)),
        2 => Some(("G726-32", 8000)),
        8 => Some(("PCMA", 8000)),
        9 => Some(("G722", 8000)),
        18 => Some(("G729", 8000)),
        _ => None,
    }
}

/// How a codec is written in SDP: payload type, name, RTP clock rate.
/// G.722's RTP clock is 8000 by the well-known RFC 3551 erratum.
pub fn codec_format(c: Codec) -> (u8, &'static str, u32) {
    match c {
        Codec::Pcmu => (0, "PCMU", 8000),
        Codec::Pcma => (8, "PCMA", 8000),
        Codec::G722 => (9, "G722", 8000),
        Codec::G726_32 => (2, "G726-32", 8000),
        Codec::G729 => (18, "G729", 8000),
        Codec::Ilbc => (97, "iLBC", 8000),
        Codec::Opus => (107, "opus", 48000),
        Codec::L16 => (117, "L16", 16000),
        Codec::L16Wide => (118, "L16", 32000),
    }
}

fn codec_of(f: &Format) -> Option<Codec> {
    let n = f.name.to_ascii_lowercase();
    Some(match (n.as_str(), f.clock) {
        ("pcmu", 8000) => Codec::Pcmu,
        ("pcma", 8000) => Codec::Pcma,
        ("g722", 8000) => Codec::G722,
        ("g726-32", 8000) => Codec::G726_32,
        ("g729", 8000) => Codec::G729,
        ("ilbc", 8000) => Codec::Ilbc,
        ("opus", 48000) => Codec::Opus,
        ("l16", 16000) => Codec::L16,
        ("l16", 32000) => Codec::L16Wide,
        _ => return None,
    })
}

impl Sdp {
    pub fn parse(text: &str) -> Result<Sdp, String> {
        let mut session_addr: Option<IpAddr> = None;
        let mut session_dir: Option<Direction> = None;
        let (mut session_id, mut version) = (0u64, 0u64);
        let mut audio: Option<Sdp> = None;
        let mut in_audio = false;
        let mut seen_media = false;
        let mut other_media = Vec::new();
        for line in text.lines() {
            let line = line.trim_end();
            let Some((kind, value)) = line.split_once('=') else { continue };
            match kind {
                "o" => {
                    let mut f = value.split_whitespace();
                    session_id = f.nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                    version = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                }
                "c" => {
                    let ip = value.split_whitespace().nth(2).and_then(|a| {
                        a.split('/').next().unwrap_or(a).parse::<IpAddr>().ok()
                    });
                    if !seen_media {
                        session_addr = ip;
                    } else if in_audio {
                        if let (Some(a), Some(ip)) = (audio.as_mut(), ip) {
                            a.addr = ip;
                        }
                    }
                }
                "m" => {
                    seen_media = true;
                    let mut f = value.split_whitespace();
                    let media = f.next().unwrap_or("");
                    if media == "audio" && audio.is_none() {
                        in_audio = true;
                        let port = f.next().and_then(|p| p.split('/').next()?.parse().ok());
                        let proto = f.next().unwrap_or("");
                        let port = port.ok_or("bad audio port")?;
                        let pts: Vec<u8> = f.filter_map(|p| p.parse().ok()).collect();
                        audio = Some(Sdp {
                            session_id,
                            version,
                            addr: session_addr.unwrap_or(IpAddr::from([0, 0, 0, 0])),
                            port,
                            secure: proto.eq_ignore_ascii_case("RTP/SAVP"),
                            formats: pts
                                .iter()
                                .map(|&pt| {
                                    let (name, clock) = static_format(pt).unwrap_or(("", 0));
                                    Format { pt, name: name.into(), clock, fmtp: String::new() }
                                })
                                .collect(),
                            direction: Direction::SendRecv,
                            ptime: None,
                            crypto: Vec::new(),
                            other_media: Vec::new(),
                        });
                        if !proto.eq_ignore_ascii_case("RTP/AVP") && !proto.eq_ignore_ascii_case("RTP/SAVP") {
                            return Err(format!("unsupported media protocol {proto}"));
                        }
                    } else {
                        in_audio = false;
                        other_media.push(value.to_string());
                    }
                }
                "a" => {
                    let (attr, arg) = value.split_once(':').unwrap_or((value, ""));
                    let dir = match attr {
                        "sendrecv" => Some(Direction::SendRecv),
                        "sendonly" => Some(Direction::SendOnly),
                        "recvonly" => Some(Direction::RecvOnly),
                        "inactive" => Some(Direction::Inactive),
                        _ => None,
                    };
                    if let Some(d) = dir {
                        if !seen_media {
                            session_dir = Some(d);
                        } else if in_audio {
                            if let Some(a) = audio.as_mut() {
                                a.direction = d;
                            }
                        }
                        continue;
                    }
                    if !in_audio {
                        continue;
                    }
                    let Some(a) = audio.as_mut() else { continue };
                    match attr {
                        "rtpmap" => {
                            let Some((pt, enc)) = arg.split_once(' ') else { continue };
                            let Ok(pt) = pt.trim().parse::<u8>() else { continue };
                            let mut e = enc.trim().split('/');
                            let name = e.next().unwrap_or("").to_string();
                            let clock = e.next().and_then(|c| c.parse().ok()).unwrap_or(8000);
                            if let Some(f) = a.formats.iter_mut().find(|f| f.pt == pt) {
                                f.name = name;
                                f.clock = clock;
                            }
                        }
                        "fmtp" => {
                            let Some((pt, p)) = arg.split_once(' ') else { continue };
                            let Ok(pt) = pt.trim().parse::<u8>() else { continue };
                            if let Some(f) = a.formats.iter_mut().find(|f| f.pt == pt) {
                                f.fmtp = p.trim().to_string();
                            }
                        }
                        "ptime" => a.ptime = arg.trim().parse().ok(),
                        "crypto" => {
                            let mut f = arg.split_whitespace();
                            let tag = f.next().and_then(|t| t.parse().ok());
                            let suite = f.next().and_then(SrtpSuite::from_name);
                            let key = f.next().and_then(|k| k.strip_prefix("inline:")).and_then(|k| {
                                let k = k.split('|').next().unwrap_or(k);
                                base64::engine::general_purpose::STANDARD
                                    .decode(k)
                                    .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(k))
                                    .ok()
                            });
                            if let (Some(tag), Some(suite), Some(key)) = (tag, suite, key) {
                                if let Ok(key_salt) = <[u8; 30]>::try_from(key.as_slice()) {
                                    a.crypto.push(Crypto { tag, suite, key_salt });
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        let mut a = audio.ok_or("no audio stream")?;
        if let (Some(d), Direction::SendRecv) = (session_dir, a.direction) {
            // A session-level direction applies unless the media overrides
            // it; an explicit media-level sendrecv is indistinguishable
            // here and rare.
            if !text.lines().skip_while(|l| !l.starts_with("m=audio")).any(|l| l.trim_end() == "a=sendrecv") {
                a.direction = d;
            }
        }
        // RFC 2543 hold: connection address 0.0.0.0.
        if a.addr.is_unspecified() && a.direction == Direction::SendRecv {
            a.direction = Direction::SendOnly;
        }
        a.other_media = other_media;
        Ok(a)
    }

    pub fn render(&self) -> String {
        let fam = if self.addr.is_ipv4() { "IP4" } else { "IP6" };
        let mut s = String::new();
        let _ = write!(
            s,
            "v=0\r\no=- {} {} IN {fam} {}\r\ns=Open Desk Phone\r\nc=IN {fam} {}\r\nt=0 0\r\n",
            self.session_id, self.version, self.addr, self.addr
        );
        let _ = write!(s, "m=audio {} {}", self.port, if self.secure { "RTP/SAVP" } else { "RTP/AVP" });
        for f in &self.formats {
            let _ = write!(s, " {}", f.pt);
        }
        s.push_str("\r\n");
        for f in &self.formats {
            let _ = write!(s, "a=rtpmap:{} {}/{}\r\n", f.pt, f.name, f.clock);
            if !f.fmtp.is_empty() {
                let _ = write!(s, "a=fmtp:{} {}\r\n", f.pt, f.fmtp);
            }
        }
        for c in &self.crypto {
            let key = base64::engine::general_purpose::STANDARD.encode(c.key_salt);
            let _ = write!(s, "a=crypto:{} {} inline:{}\r\n", c.tag, c.suite.name(), key);
        }
        if let Some(p) = self.ptime {
            let _ = write!(s, "a=ptime:{p}\r\n");
        }
        let _ = write!(s, "a={}\r\n", self.direction.token());
        for m in &self.other_media {
            // Refused streams: same line with port zero (RFC 3264 section 6).
            let mut f = m.split_whitespace();
            let (media, _port) = (f.next().unwrap_or("video"), f.next());
            let rest: Vec<&str> = f.collect();
            let _ = write!(s, "m={media} 0 {}\r\n", rest.join(" "));
        }
        s
    }
}

/// SRTP policy for a call (per-host `media_encryption`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SrtpPolicy {
    /// Plain RTP only.
    #[default]
    Off,
    /// SDES is required: offer `RTP/SAVP`, refuse plain offers.
    Sdes,
}

/// What we bring to a negotiation.
#[derive(Clone, Debug)]
pub struct Local {
    pub addr: IpAddr,
    pub port: u16,
    /// Enabled codecs in preference order (already limited to what the
    /// media engine can run).
    pub codecs: Vec<Codec>,
    pub ptime: u16,
    pub srtp: SrtpPolicy,
    pub direction: Direction,
    pub session_id: u64,
    pub version: u64,
}

/// SRTP keys of an agreed stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SrtpKeys {
    pub suite: SrtpSuite,
    pub tx: [u8; 30],
    pub rx: [u8; 30],
}

/// The outcome of offer/answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Negotiated {
    pub codec: Codec,
    /// Payload type to send the codec with (the remote's number).
    pub tx_pt: u8,
    /// Payload type we receive it with.
    pub rx_pt: u8,
    pub remote: SocketAddr,
    /// Telephone-event payload type to send, if agreed.
    pub dtmf_tx_pt: Option<u8>,
    pub dtmf_rx_pt: Option<u8>,
    pub ptime: u16,
    pub srtp: Option<SrtpKeys>,
    /// We may send / expect to receive.
    pub send: bool,
    pub recv: bool,
}

fn dtmf_format(pt: u8) -> Format {
    Format { pt, name: "telephone-event".into(), clock: 8000, fmtp: "0-16".into() }
}

fn find_dtmf(sdp: &Sdp) -> Option<u8> {
    sdp.formats
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case("telephone-event") && f.clock == 8000)
        .map(|f| f.pt)
}

/// Our offer.  With SDES, `key` is the key we send with.
pub fn offer(local: &Local, key: Option<&Crypto>) -> Sdp {
    let mut formats: Vec<Format> = local
        .codecs
        .iter()
        .map(|&c| {
            let (pt, name, clock) = codec_format(c);
            Format { pt, name: name.into(), clock, fmtp: String::new() }
        })
        .collect();
    formats.push(dtmf_format(DTMF_PT));
    Sdp {
        session_id: local.session_id,
        version: local.version,
        addr: local.addr,
        port: local.port,
        secure: local.srtp == SrtpPolicy::Sdes,
        formats,
        direction: local.direction,
        ptime: Some(local.ptime),
        crypto: match (local.srtp, key) {
            (SrtpPolicy::Sdes, Some(k)) => vec![k.clone()],
            _ => Vec::new(),
        },
        other_media: Vec::new(),
    }
}

/// Answers `offer`.  Returns our answer and what was agreed.  The codec is
/// the first of *our* preference order that the offer contains.
pub fn answer(remote: &Sdp, local: &Local, key: Option<&Crypto>) -> Result<(Sdp, Negotiated), String> {
    if remote.port == 0 {
        return Err("the audio stream is disabled".into());
    }
    let (codec, their) = local
        .codecs
        .iter()
        .find_map(|&c| remote.formats.iter().find(|f| codec_of(f) == Some(c)).map(|f| (c, f)))
        .ok_or("no common codec")?;
    let srtp = match (local.srtp, remote.secure) {
        (SrtpPolicy::Off, false) => None,
        (SrtpPolicy::Off, true) => return Err("SRTP offered but not enabled for this account".into()),
        (SrtpPolicy::Sdes, false) => return Err("SRTP is required for this account".into()),
        (SrtpPolicy::Sdes, true) => {
            let theirs = remote.crypto.first().ok_or("RTP/SAVP offer without a usable crypto line")?;
            let ours = key.ok_or("no local SRTP key")?;
            Some((theirs.clone(), Crypto { tag: theirs.tag, suite: theirs.suite, key_salt: ours.key_salt }))
        }
    };
    let direction = remote.direction.answer(local.direction);
    let dtmf = find_dtmf(remote);
    let mut formats = vec![Format {
        pt: their.pt,
        name: their.name.clone(),
        clock: their.clock,
        fmtp: String::new(),
    }];
    if let Some(pt) = dtmf {
        formats.push(dtmf_format(pt));
    }
    let ptime = remote.ptime.filter(|p| *p >= 10 && *p <= 60).unwrap_or(local.ptime);
    let ans = Sdp {
        session_id: local.session_id,
        version: local.version,
        addr: local.addr,
        port: local.port,
        secure: srtp.is_some(),
        formats,
        direction,
        ptime: Some(ptime),
        crypto: srtp.iter().map(|(_, ours)| ours.clone()).collect(),
        other_media: remote.other_media.clone(),
    };
    let neg = Negotiated {
        codec,
        tx_pt: their.pt,
        rx_pt: their.pt,
        remote: SocketAddr::new(remote.addr, remote.port),
        dtmf_tx_pt: dtmf,
        dtmf_rx_pt: dtmf,
        ptime,
        srtp: srtp.map(|(theirs, ours)| SrtpKeys { suite: theirs.suite, tx: ours.key_salt, rx: theirs.key_salt }),
        send: direction.sends() && !remote.addr.is_unspecified(),
        recv: direction.receives(),
    };
    Ok((ans, neg))
}

/// Applies the remote's answer to our `offer`.
pub fn accept(ours: &Sdp, remote: &Sdp) -> Result<Negotiated, String> {
    if remote.port == 0 {
        return Err("the audio stream was refused".into());
    }
    // The first format of the answer that is a codec we offered.
    let (codec, theirs, mine) = remote
        .formats
        .iter()
        .find_map(|f| {
            let c = codec_of(f)?;
            let m = ours.formats.iter().find(|o| codec_of(o) == Some(c))?;
            Some((c, f, m))
        })
        .ok_or("the answer has no codec that was offered")?;
    let srtp = if ours.secure {
        if !remote.secure {
            return Err("the answer dropped SRTP".into());
        }
        let k = ours.crypto.first().ok_or("no local SRTP key")?;
        let r = remote
            .crypto
            .iter()
            .find(|c| c.tag == k.tag)
            .or(remote.crypto.first())
            .ok_or("the answer has no crypto line")?;
        Some(SrtpKeys { suite: r.suite, tx: k.key_salt, rx: r.key_salt })
    } else if remote.secure {
        return Err("the answer uses SRTP but the offer did not".into());
    } else {
        None
    };
    let dir = remote.direction;
    Ok(Negotiated {
        codec,
        tx_pt: theirs.pt,
        rx_pt: mine.pt,
        remote: SocketAddr::new(remote.addr, remote.port),
        dtmf_tx_pt: find_dtmf(remote),
        dtmf_rx_pt: find_dtmf(ours),
        ptime: remote.ptime.filter(|p| *p >= 10 && *p <= 60).or(ours.ptime).unwrap_or(20),
        srtp,
        // Their "recvonly" means we may send.
        send: dir.receives() && ours.direction.sends() && !remote.addr.is_unspecified(),
        recv: dir.sends() && ours.direction.receives(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(codecs: &[Codec], srtp: SrtpPolicy) -> Local {
        Local {
            addr: "192.0.2.10".parse().unwrap(),
            port: 4000,
            codecs: codecs.to_vec(),
            ptime: 20,
            srtp,
            direction: Direction::SendRecv,
            session_id: 1,
            version: 1,
        }
    }

    const ASTERISK: &str = "v=0\r\no=- 123 456 IN IP4 198.51.100.1\r\ns=Asterisk\r\nc=IN IP4 198.51.100.1\r\nt=0 0\r\n\
        m=audio 10000 RTP/AVP 9 0 8 101\r\na=rtpmap:9 G722/8000\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\n\
        a=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-16\r\na=ptime:20\r\na=maxptime:150\r\na=sendrecv\r\n\
        m=video 0 RTP/AVP 99\r\n";

    #[test]
    fn parse_and_answer() {
        let offer = Sdp::parse(ASTERISK).unwrap();
        assert_eq!(offer.port, 10000);
        assert_eq!(offer.formats.len(), 4);
        assert_eq!(offer.other_media.len(), 1);
        let (ans, neg) = answer(&offer, &local(&[Codec::Pcmu, Codec::G722], SrtpPolicy::Off), None).unwrap();
        assert_eq!(neg.codec, Codec::Pcmu, "our preference order wins");
        assert_eq!(neg.dtmf_tx_pt, Some(101));
        assert_eq!(neg.remote, "198.51.100.1:10000".parse().unwrap());
        assert!(neg.send && neg.recv);
        let text = ans.render();
        assert!(text.contains("m=audio 4000 RTP/AVP 0 101\r\n"), "{text}");
        assert!(text.contains("m=video 0 RTP/AVP 99\r\n"), "{text}");
        // And the rendered answer parses back.
        assert_eq!(Sdp::parse(&text).unwrap().formats.len(), 2);
    }

    #[test]
    fn offer_accept_round_trip() {
        let l = local(&[Codec::G722, Codec::Pcmu, Codec::Pcma, Codec::G726_32], SrtpPolicy::Off);
        let o = offer(&l, None);
        let text = o.render();
        assert!(text.contains("a=rtpmap:2 G726-32/8000"));
        let remote_local = Local { addr: "192.0.2.20".parse().unwrap(), port: 5000, ..local(&[Codec::Pcma], SrtpPolicy::Off) };
        let (ans, n1) = answer(&Sdp::parse(&text).unwrap(), &remote_local, None).unwrap();
        let n2 = accept(&o, &Sdp::parse(&ans.render()).unwrap()).unwrap();
        assert_eq!(n1.codec, Codec::Pcma);
        assert_eq!(n2.codec, Codec::Pcma);
        assert_eq!(n2.remote, "192.0.2.20:5000".parse().unwrap());
        assert_eq!(n2.dtmf_tx_pt, Some(DTMF_PT));
    }

    #[test]
    fn hold_directions() {
        let mut held = Sdp::parse(ASTERISK).unwrap();
        held.direction = Direction::SendOnly;
        let (ans, neg) = answer(&held, &local(&[Codec::Pcmu], SrtpPolicy::Off), None).unwrap();
        assert_eq!(ans.direction, Direction::RecvOnly);
        assert!(!neg.send && neg.recv);
        // Old-style hold.
        let old = ASTERISK.replace("c=IN IP4 198.51.100.1", "c=IN IP4 0.0.0.0");
        assert_eq!(Sdp::parse(&old).unwrap().direction, Direction::SendOnly);
        // We hold: sendonly offer, they answer recvonly, nobody sends to us.
        let mut l = local(&[Codec::Pcmu], SrtpPolicy::Off);
        l.direction = Direction::SendOnly;
        let o = offer(&l, None);
        let mut a = Sdp::parse(ASTERISK).unwrap();
        a.direction = Direction::RecvOnly;
        let n = accept(&o, &a).unwrap();
        assert!(n.send && !n.recv);
    }

    #[test]
    fn sdes() {
        let l = local(&[Codec::Pcmu], SrtpPolicy::Sdes);
        let k1 = Crypto { tag: 1, suite: SrtpSuite::AesCm128HmacSha1_80, key_salt: [1; 30] };
        let k2 = Crypto { tag: 1, suite: SrtpSuite::AesCm128HmacSha1_80, key_salt: [2; 30] };
        let o = offer(&l, Some(&k1));
        let text = o.render();
        assert!(text.contains("RTP/SAVP"));
        assert!(text.contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:AQEB"));
        let (ans, n1) = answer(&Sdp::parse(&text).unwrap(), &l, Some(&k2)).unwrap();
        let n2 = accept(&o, &Sdp::parse(&ans.render()).unwrap()).unwrap();
        let (s1, s2) = (n1.srtp.unwrap(), n2.srtp.unwrap());
        assert_eq!(s1.tx, [2; 30]);
        assert_eq!(s1.rx, [1; 30]);
        assert_eq!(s2.tx, s1.rx);
        assert_eq!(s2.rx, s1.tx);
        // Policy mismatches fail.
        assert!(answer(&Sdp::parse(ASTERISK).unwrap(), &l, Some(&k2)).is_err());
        assert!(answer(&Sdp::parse(&text).unwrap(), &local(&[Codec::Pcmu], SrtpPolicy::Off), None).is_err());
    }

    #[test]
    fn no_common_codec() {
        let offer = Sdp::parse(ASTERISK).unwrap();
        assert!(answer(&offer, &local(&[Codec::G726_32], SrtpPolicy::Off), None).is_err());
    }
}
