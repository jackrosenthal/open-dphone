// SPDX-License-Identifier: GPL-2.0-or-later
//! RTP and RTCP (RFC 3550): headers, receive statistics, and the reports a
//! phone sends.

pub const RTP_HEADER: usize = 12;
/// RTP timestamp units per 10 ms for every codec handled here: the 8 kHz
/// clock, which RTP also uses for G.722 despite its 16 kHz audio.
pub const TS_PER_TICK: u32 = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtpHeader {
    pub marker: bool,
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
}

/// Parses a packet; returns the header and the payload (padding and any
/// CSRC list or header extension removed).
pub fn parse(pkt: &[u8]) -> Option<(RtpHeader, &[u8])> {
    if pkt.len() < RTP_HEADER || pkt[0] >> 6 != 2 {
        return None;
    }
    let cc = usize::from(pkt[0] & 0x0f);
    let mut off = RTP_HEADER + 4 * cc;
    if pkt[0] & 0x10 != 0 {
        if pkt.len() < off + 4 {
            return None;
        }
        off += 4 + 4 * usize::from(u16::from_be_bytes([pkt[off + 2], pkt[off + 3]]));
    }
    let mut end = pkt.len();
    if pkt[0] & 0x20 != 0 {
        let pad = usize::from(*pkt.last()?);
        if pad == 0 || pad > end.saturating_sub(off) {
            return None;
        }
        end -= pad;
    }
    if off > end {
        return None;
    }
    Some((
        RtpHeader {
            marker: pkt[1] & 0x80 != 0,
            pt: pkt[1] & 0x7f,
            seq: u16::from_be_bytes([pkt[2], pkt[3]]),
            ts: u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]),
            ssrc: u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]),
        },
        &pkt[off..end],
    ))
}

/// Writes a 12-byte header to the start of `buf`.
pub fn write_header(buf: &mut [u8], h: &RtpHeader) {
    buf[0] = 0x80;
    buf[1] = (h.pt & 0x7f) | if h.marker { 0x80 } else { 0 };
    buf[2..4].copy_from_slice(&h.seq.to_be_bytes());
    buf[4..8].copy_from_slice(&h.ts.to_be_bytes());
    buf[8..12].copy_from_slice(&h.ssrc.to_be_bytes());
}

/// Receive-side sequence tracking, loss, and jitter (RFC 3550 appendix A).
#[derive(Clone, Debug, Default)]
pub struct RxStats {
    started: bool,
    pub ssrc: u32,
    base_seq: u32,
    max_seq: u16,
    cycles: u32,
    pub received: u32,
    expected_prior: u32,
    received_prior: u32,
    /// Interarrival jitter in timestamp units, Q4.
    jitter_q4: u32,
    last_transit: i32,
    pub last_sr_ntp_mid: u32,
    /// Local time (timestamp units) when the last sender report arrived.
    pub last_sr_at: u32,
}

/// One reception report block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReportBlock {
    pub ssrc: u32,
    pub fraction_lost: u8,
    /// Cumulative packets lost, 24 bits, signed.
    pub lost: i32,
    pub highest_seq: u32,
    pub jitter: u32,
    pub lsr: u32,
    pub dlsr: u32,
}

impl RxStats {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Accounts for a packet.  `arrival` is local time in timestamp units.
    pub fn update(&mut self, h: &RtpHeader, arrival: u32) {
        if !self.started || h.ssrc != self.ssrc {
            *self = Self { started: true, ssrc: h.ssrc, base_seq: u32::from(h.seq), max_seq: h.seq, ..Self::default() };
            self.last_transit = arrival.wrapping_sub(h.ts) as i32;
        } else {
            let delta = h.seq.wrapping_sub(self.max_seq);
            if delta != 0 && delta < 0x8000 {
                if h.seq < self.max_seq {
                    self.cycles += 1 << 16;
                }
                self.max_seq = h.seq;
            }
        }
        self.received += 1;
        let transit = arrival.wrapping_sub(h.ts) as i32;
        let d = transit.wrapping_sub(self.last_transit).unsigned_abs();
        self.last_transit = transit;
        // J += (|D| - J) / 16, kept in Q4
        self.jitter_q4 = self.jitter_q4.wrapping_add(d).wrapping_sub((self.jitter_q4 + 8) >> 4);
    }

    pub fn jitter(&self) -> u32 {
        self.jitter_q4 >> 4
    }

    pub fn highest_seq(&self) -> u32 {
        self.cycles | u32::from(self.max_seq)
    }

    pub fn expected(&self) -> u32 {
        if !self.started { 0 } else { self.highest_seq().wrapping_sub(self.base_seq).wrapping_add(1) }
    }

    pub fn lost(&self) -> i32 {
        self.expected() as i32 - self.received as i32
    }

    /// A report block covering the interval since the previous call.
    pub fn report(&mut self, now: u32) -> ReportBlock {
        let expected = self.expected();
        let exp_int = expected.wrapping_sub(self.expected_prior);
        let rec_int = self.received.wrapping_sub(self.received_prior);
        self.expected_prior = expected;
        self.received_prior = self.received;
        let lost_int = exp_int as i32 - rec_int as i32;
        let fraction_lost = if exp_int == 0 || lost_int <= 0 { 0 } else { ((lost_int as u32) << 8).checked_div(exp_int).unwrap_or(0).min(255) as u8 };
        let dlsr = if self.last_sr_ntp_mid == 0 {
            0
        } else {
            // timestamp units (1/8000 s) to 1/65536 s
            (u64::from(now.wrapping_sub(self.last_sr_at)) * 65536 / 8000) as u32
        };
        ReportBlock {
            ssrc: self.ssrc,
            fraction_lost,
            lost: self.lost().clamp(-0x80_0000, 0x7f_ffff),
            highest_seq: self.highest_seq(),
            jitter: self.jitter(),
            lsr: self.last_sr_ntp_mid,
            dlsr,
        }
    }
}

pub const RTCP_SR: u8 = 200;
pub const RTCP_RR: u8 = 201;
pub const RTCP_SDES: u8 = 202;
pub const RTCP_BYE: u8 = 203;

fn put_block(out: &mut Vec<u8>, b: &ReportBlock) {
    out.extend_from_slice(&b.ssrc.to_be_bytes());
    out.push(b.fraction_lost);
    out.extend_from_slice(&(b.lost as u32).to_be_bytes()[1..]);
    out.extend_from_slice(&b.highest_seq.to_be_bytes());
    out.extend_from_slice(&b.jitter.to_be_bytes());
    out.extend_from_slice(&b.lsr.to_be_bytes());
    out.extend_from_slice(&b.dlsr.to_be_bytes());
}

fn put_sdes(out: &mut Vec<u8>, ssrc: u32, cname: &str) {
    let cname = &cname.as_bytes()[..cname.len().min(255)];
    let start = out.len();
    out.extend_from_slice(&[0x81, RTCP_SDES, 0, 0]);
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.push(1);
    out.push(cname.len() as u8);
    out.extend_from_slice(cname);
    out.push(0);
    while !(out.len() - start).is_multiple_of(4) {
        out.push(0);
    }
    let words = ((out.len() - start) / 4 - 1) as u16;
    out[start + 2..start + 4].copy_from_slice(&words.to_be_bytes());
}

/// What a sender report says about the sender.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SenderInfo {
    /// NTP timestamp, seconds since 1900 in the high word.
    pub ntp: u64,
    pub rtp_ts: u32,
    pub packets: u32,
    pub octets: u32,
}

/// Builds a compound packet into `out` (cleared first): a sender report (or
/// a receiver report when `sender` is `None`), an SDES with the CNAME, and
/// optionally a BYE.
pub fn build_report(out: &mut Vec<u8>, ssrc: u32, sender: Option<&SenderInfo>, block: Option<&ReportBlock>, cname: &str, bye: bool) {
    out.clear();
    let rc = u8::from(block.is_some());
    match sender {
        Some(s) => {
            out.extend_from_slice(&[0x80 | rc, RTCP_SR, 0, 6 + 6 * u16::from(rc) as u8]);
            out.extend_from_slice(&ssrc.to_be_bytes());
            out.extend_from_slice(&s.ntp.to_be_bytes());
            out.extend_from_slice(&s.rtp_ts.to_be_bytes());
            out.extend_from_slice(&s.packets.to_be_bytes());
            out.extend_from_slice(&s.octets.to_be_bytes());
        }
        None => {
            out.extend_from_slice(&[0x80 | rc, RTCP_RR, 0, 1 + 6 * rc]);
            out.extend_from_slice(&ssrc.to_be_bytes());
        }
    }
    if let Some(b) = block {
        put_block(out, b);
    }
    put_sdes(out, ssrc, cname);
    if bye {
        out.extend_from_slice(&[0x81, RTCP_BYE, 0, 1]);
        out.extend_from_slice(&ssrc.to_be_bytes());
    }
}

/// One packet of a compound RTCP packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtcpItem {
    SenderReport { ssrc: u32, info: SenderInfo, block: Option<ReportBlock> },
    ReceiverReport { ssrc: u32, block: Option<ReportBlock> },
    Bye { ssrc: u32 },
    Other(u8),
}

fn get_block(b: &[u8]) -> Option<ReportBlock> {
    if b.len() < 24 {
        return None;
    }
    let w = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    // sign-extend the 24-bit loss count
    let lost = ((w(4) << 8) as i32) >> 8;
    Some(ReportBlock { ssrc: w(0), fraction_lost: b[4], lost, highest_seq: w(8), jitter: w(12), lsr: w(16), dlsr: w(20) })
}

/// Walks a compound RTCP packet.  Malformed input ends the walk.
pub fn parse_rtcp(mut pkt: &[u8], mut each: impl FnMut(RtcpItem)) {
    while pkt.len() >= 4 && pkt[0] >> 6 == 2 {
        let len = 4 * (usize::from(u16::from_be_bytes([pkt[2], pkt[3]])) + 1);
        if len > pkt.len() {
            return;
        }
        let (p, rest) = pkt.split_at(len);
        let rc = p[0] & 0x1f;
        let w = |o: usize| u32::from_be_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
        match p[1] {
            RTCP_SR if len >= 28 => each(RtcpItem::SenderReport {
                ssrc: w(4),
                info: SenderInfo { ntp: (u64::from(w(8)) << 32) | u64::from(w(12)), rtp_ts: w(16), packets: w(20), octets: w(24) },
                block: if rc > 0 { get_block(&p[28..]) } else { None },
            }),
            RTCP_RR if len >= 8 => each(RtcpItem::ReceiverReport { ssrc: w(4), block: if rc > 0 { get_block(&p[8..]) } else { None } }),
            RTCP_BYE if len >= 8 => each(RtcpItem::Bye { ssrc: w(4) }),
            other => each(RtcpItem::Other(other)),
        }
        pkt = rest;
    }
}

/// NTP time now (seconds since 1900 in the high 32 bits).
pub fn ntp_now() -> u64 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    ((d.as_secs() + 2_208_988_800) << 32) | ((u64::from(d.subsec_nanos()) << 32) / 1_000_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trip() {
        let h = RtpHeader { marker: true, pt: 9, seq: 0xfffe, ts: 0x1234_5678, ssrc: 0xdead_beef };
        let mut pkt = [0u8; 16];
        write_header(&mut pkt, &h);
        pkt[12..].copy_from_slice(&[1, 2, 3, 4]);
        let (g, payload) = parse(&pkt).unwrap();
        assert_eq!(g, h);
        assert_eq!(payload, [1, 2, 3, 4]);
        assert!(parse(&pkt[..8]).is_none());
        let mut bad = pkt;
        bad[0] = 0x40;
        assert!(parse(&bad).is_none());
    }

    #[test]
    fn csrc_extension_padding() {
        let mut pkt = vec![0xb1, 0x00, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3];
        pkt.extend_from_slice(&[9, 9, 9, 9]); // one CSRC
        pkt.extend_from_slice(&[0xbe, 0xde, 0, 1, 7, 7, 7, 7]); // extension, one word
        pkt.extend_from_slice(&[0xaa, 0xbb, 0, 0, 0, 4]); // payload aa bb + 4 bytes padding
        let (h, payload) = parse(&pkt).unwrap();
        assert_eq!((h.pt, h.seq, h.ts, h.ssrc), (0, 1, 2, 3));
        assert_eq!(payload, [0xaa, 0xbb]);
    }

    #[test]
    fn loss_and_wrap() {
        let mut s = RxStats::default();
        let mut now = 0u32;
        let mut feed = |s: &mut RxStats, seq: u16| {
            now += 160;
            s.update(&RtpHeader { marker: false, pt: 0, seq, ts: now, ssrc: 7 }, now + 5);
        };
        for seq in [65533u16, 65534, 65535, 0, 1, 3, 4] {
            feed(&mut s, seq);
        }
        assert_eq!(s.expected(), 8);
        assert_eq!(s.lost(), 1);
        assert_eq!(s.highest_seq(), 0x1_0004);
        let b = s.report(now);
        assert_eq!(b.fraction_lost, 32); // 1 of 8
        assert_eq!(b.lost, 1);
        // steady arrival: jitter stays tiny
        assert!(s.jitter() <= 1);
    }

    #[test]
    fn rtcp_round_trip() {
        let block = ReportBlock { ssrc: 5, fraction_lost: 3, lost: -2, highest_seq: 99, jitter: 4, lsr: 6, dlsr: 7 };
        let info = SenderInfo { ntp: 0x0102_0304_0506_0708, rtp_ts: 11, packets: 12, octets: 13 };
        let mut out = Vec::new();
        build_report(&mut out, 42, Some(&info), Some(&block), "phone@example", true);
        assert_eq!(out.len() % 4, 0);
        let mut items = Vec::new();
        parse_rtcp(&out, |i| items.push(i));
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], RtcpItem::SenderReport { ssrc: 42, info, block: Some(block) });
        assert_eq!(items[1], RtcpItem::Other(RTCP_SDES));
        assert_eq!(items[2], RtcpItem::Bye { ssrc: 42 });
        build_report(&mut out, 42, None, None, "x", false);
        let mut items = Vec::new();
        parse_rtcp(&out, |i| items.push(i));
        assert_eq!(items[0], RtcpItem::ReceiverReport { ssrc: 42, block: None });
    }
}
