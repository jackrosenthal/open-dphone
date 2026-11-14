// SPDX-License-Identifier: GPL-2.0-or-later
//! RFC 4733 telephone events (DTMF), both directions.

/// Event payload length.
pub const EVENT_LEN: usize = 4;

/// Event code of a key: `0`-`9`, `*`, `#`, `A`-`D`; `!` is hook flash.
pub fn event_code(key: char) -> Option<u8> {
    Some(match key.to_ascii_uppercase() {
        k @ '0'..='9' => k as u8 - b'0',
        '*' => 10,
        '#' => 11,
        k @ 'A'..='D' => 12 + (k as u8 - b'A'),
        '!' => 16,
        _ => return None,
    })
}

pub fn event_key(code: u8) -> Option<char> {
    Some(match code {
        0..=9 => (b'0' + code) as char,
        10 => '*',
        11 => '#',
        12..=15 => (b'A' + code - 12) as char,
        16 => '!',
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventPayload {
    pub event: u8,
    pub end: bool,
    /// Power level in -dBm0, 0..=63.
    pub volume: u8,
    /// In timestamp units.
    pub duration: u16,
}

impl EventPayload {
    pub fn parse(p: &[u8]) -> Option<Self> {
        if p.len() < EVENT_LEN {
            return None;
        }
        Some(Self { event: p[0], end: p[1] & 0x80 != 0, volume: p[1] & 0x3f, duration: u16::from_be_bytes([p[2], p[3]]) })
    }

    pub fn write(&self, out: &mut [u8]) {
        out[0] = self.event;
        out[1] = (self.volume & 0x3f) | if self.end { 0x80 } else { 0 };
        out[2..4].copy_from_slice(&self.duration.to_be_bytes());
    }
}

/// What the receiver reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DtmfEdge {
    Start(char),
    /// The event ended; its duration in milliseconds.
    End(char, u32),
}

/// Turns the packets of incoming events into one start and one end each.
/// An event is identified by its RTP timestamp; retransmitted end packets
/// and reordered packets are ignored.
#[derive(Clone, Debug, Default)]
pub struct DtmfReceiver {
    current: Option<(u32, u8)>,
    ended: Option<u32>,
    /// Ticks since the last packet of the current event.
    idle_ticks: u32,
}

impl DtmfReceiver {
    /// Feeds one event packet; up to two edges come back (the end of an
    /// event whose end packets were lost, then the start of the next).
    pub fn packet(&mut self, ts: u32, p: &EventPayload) -> [Option<DtmfEdge>; 2] {
        let mut out = [None, None];
        let Some(key) = event_key(p.event) else { return out };
        if self.ended == Some(ts) {
            return out;
        }
        self.idle_ticks = 0;
        match self.current {
            Some((cur_ts, _)) if cur_ts == ts => {}
            other => {
                if let Some((_, ev)) = other {
                    out[0] = event_key(ev).map(|k| DtmfEdge::End(k, 0));
                }
                self.current = Some((ts, p.event));
                out[1] = Some(DtmfEdge::Start(key));
            }
        }
        if p.end {
            self.current = None;
            self.ended = Some(ts);
            let edge = Some(DtmfEdge::End(key, u32::from(p.duration) / 8));
            if out[1].is_some() && out[0].is_none() {
                out[0] = out[1].take();
                out[1] = edge;
            } else if out[1].is_none() {
                out[0] = edge;
            } else {
                // previous event's lost end, this event's start; its own end
                // follows from the retransmitted end packet's timeout below
                self.current = Some((ts, p.event));
                self.ended = None;
            }
        }
        out
    }

    /// Call every 10 ms.  Ends an event whose packets stopped coming
    /// without an end packet.
    pub fn tick(&mut self) -> Option<DtmfEdge> {
        let (_, ev) = self.current?;
        self.idle_ticks += 1;
        if self.idle_ticks > 10 {
            self.current = None;
            return event_key(ev).map(|k| DtmfEdge::End(k, 0));
        }
        None
    }
}

/// Sends one event: a packet per packet interval, all with the timestamp of
/// the event's start and a growing duration, then three end packets.
#[derive(Clone, Debug)]
pub struct DtmfSender {
    event: u8,
    /// RTP timestamp of the event's start.
    pub ts: u32,
    /// Timestamp units sent so far.
    duration: u32,
    /// Timestamp units the key is to last.
    total: u32,
    ends_left: u8,
    first: bool,
}

/// One packet to send for an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventPacket {
    pub marker: bool,
    pub ts: u32,
    pub payload: [u8; EVENT_LEN],
}

impl DtmfSender {
    pub fn new(key: char, ts: u32, duration_ms: u32) -> Option<Self> {
        Some(Self { event: event_code(key)?, ts, duration: 0, total: duration_ms.clamp(40, 8000) * 8, ends_left: 3, first: true })
    }

    /// The packet for the next interval of `interval_ts` timestamp units,
    /// or `None` when the event is over.
    pub fn next(&mut self, interval_ts: u32) -> Option<EventPacket> {
        if self.ends_left == 0 {
            return None;
        }
        let end = self.duration + interval_ts >= self.total;
        if self.ends_left == 3 {
            self.duration = (self.duration + interval_ts).min(self.total);
        }
        if end {
            self.ends_left -= 1;
        }
        let mut payload = [0u8; EVENT_LEN];
        EventPayload { event: self.event, end, volume: 10, duration: self.duration.min(0xffff) as u16 }.write(&mut payload);
        let marker = std::mem::take(&mut self.first);
        Some(EventPacket { marker, ts: self.ts, payload })
    }

    /// Timestamp units the event occupies on the sender's clock.
    pub fn total(&self) -> u32 {
        self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        for k in "0123456789*#ABCD!".chars() {
            assert_eq!(event_key(event_code(k).unwrap()), Some(k));
        }
        assert_eq!(event_code('a'), Some(12));
        assert_eq!(event_code('x'), None);
        assert_eq!(event_key(17), None);
    }

    #[test]
    fn sender_sequence() {
        let mut s = DtmfSender::new('5', 1000, 100).unwrap();
        let mut pkts = Vec::new();
        while let Some(p) = s.next(160) {
            pkts.push(p);
        }
        // 100 ms at 20 ms per packet: 5 packets growing to 800, the last of
        // them with the end bit, then two more end packets
        assert_eq!(pkts.len(), 7);
        assert!(pkts[0].marker && pkts[1..].iter().all(|p| !p.marker));
        assert!(pkts.iter().all(|p| p.ts == 1000));
        let pl: Vec<_> = pkts.iter().map(|p| EventPayload::parse(&p.payload).unwrap()).collect();
        assert_eq!(pl.iter().map(|p| p.duration).collect::<Vec<_>>(), [160, 320, 480, 640, 800, 800, 800]);
        assert_eq!(pl.iter().map(|p| p.end).collect::<Vec<_>>(), [false, false, false, false, true, true, true]);
        assert!(pl.iter().all(|p| p.event == 5 && p.volume == 10));
    }

    #[test]
    fn receiver_dedups() {
        let mut r = DtmfReceiver::default();
        let p = |end, duration| EventPayload { event: 11, end, volume: 10, duration };
        assert_eq!(r.packet(500, &p(false, 160)), [None, Some(DtmfEdge::Start('#'))]);
        assert_eq!(r.packet(500, &p(false, 320)), [None, None]);
        assert_eq!(r.packet(500, &p(true, 800)), [Some(DtmfEdge::End('#', 100)), None]);
        assert_eq!(r.packet(500, &p(true, 800)), [None, None]);
        assert_eq!(r.packet(500, &p(false, 480)), [None, None]);
        // a whole event in one end packet
        assert_eq!(r.packet(900, &p(true, 400)), [Some(DtmfEdge::Start('#')), Some(DtmfEdge::End('#', 50))]);
        // an event whose end is lost times out
        assert_eq!(r.packet(2000, &p(false, 160)), [None, Some(DtmfEdge::Start('#'))]);
        let mut ended = None;
        for _ in 0..12 {
            ended = ended.or(r.tick());
        }
        assert_eq!(ended, Some(DtmfEdge::End('#', 0)));
        assert_eq!(r.tick(), None);
    }
}
