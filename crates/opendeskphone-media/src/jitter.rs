// SPDX-License-Identifier: GPL-2.0-or-later
//! An adaptive jitter buffer.
//!
//! Audio is held in units of 10 ms (80 RTP timestamp units for every codec
//! handled here), each `unit_bytes` of payload: a packet is cut into its
//! units on arrival, and the audio thread takes one unit per tick.  Nothing
//! is allocated after construction.

/// Largest unit: 10 ms of G.711 or G.722.
pub const UNIT_MAX: usize = 80;
const SLOTS: usize = 64;
const TS_PER_UNIT: u32 = 80;
/// Pops per adaptation window (2 s).
const WINDOW: u32 = 200;
/// Ticks in a row with nothing held after which the stream counts as paused.
const PAUSE_AFTER: u32 = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pop {
    /// A unit was written to the output.
    Frame,
    /// The unit due now did not arrive.
    Lost,
    /// Nothing is due: no stream yet, still filling, or the sender paused.
    Idle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitterStats {
    /// Units that arrived after their time.
    pub late: u32,
    /// Units that were due and missing.
    pub lost: u32,
    /// Units thrown away to shorten the delay or on overflow.
    pub dropped: u32,
    /// Times the buffer started over (timestamp jump, new source).
    pub resyncs: u32,
    /// Current target delay in units.
    pub target: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Empty,
    Filling,
    Playing,
}

pub struct JitterBuffer {
    unit_bytes: usize,
    used: [bool; SLOTS],
    data: [[u8; UNIT_MAX]; SLOTS],
    /// Slot of the unit due next, and that unit's timestamp.
    base: usize,
    play_ts: u32,
    /// Units from `base` up to and including the newest one held.
    span: u32,
    state: State,
    filling_ticks: u32,
    /// Ticks in a row with nothing held.
    empty_ticks: u32,
    /// Whether the current stall was counted as a lost unit.
    stall_counted: u32,
    target: u32,
    min_target: u32,
    max_target: u32,
    wanted: u32,
    window_pops: u32,
    window_min_span: u32,
    window_trouble: bool,
    stats: JitterStats,
}

impl JitterBuffer {
    /// `unit_bytes` is the payload of 10 ms; delays are in milliseconds.
    pub fn new(unit_bytes: usize, min_ms: u32, max_ms: u32) -> Self {
        let min_target = (min_ms / 10).clamp(1, 40);
        let max_target = (max_ms / 10).clamp(min_target, 40);
        Self {
            unit_bytes: unit_bytes.clamp(1, UNIT_MAX),
            used: [false; SLOTS],
            data: [[0; UNIT_MAX]; SLOTS],
            base: 0,
            play_ts: 0,
            span: 0,
            state: State::Empty,
            filling_ticks: 0,
            empty_ticks: 0,
            stall_counted: 0,
            target: (min_target + 2).min(max_target),
            min_target,
            max_target,
            wanted: min_target,
            window_pops: 0,
            window_min_span: u32::MAX,
            window_trouble: false,
            stats: JitterStats::default(),
        }
    }

    pub fn unit_bytes(&self) -> usize {
        self.unit_bytes
    }

    pub fn stats(&self) -> JitterStats {
        JitterStats { target: self.target, ..self.stats }
    }

    /// Units held, counted from the one due next to the newest.
    pub fn depth(&self) -> u32 {
        self.span
    }

    /// Forgets everything held; the next packet starts a new stream.
    pub fn reset(&mut self) {
        self.used = [false; SLOTS];
        self.span = 0;
        self.state = State::Empty;
        self.empty_ticks = 0;
    }

    /// Tells the buffer the measured interarrival jitter (timestamp units);
    /// the target delay follows it slowly.
    pub fn set_jitter(&mut self, jitter_ts: u32) {
        self.wanted = (2 * jitter_ts).div_ceil(TS_PER_UNIT).saturating_add(1).clamp(self.min_target, self.max_target);
    }

    fn restart(&mut self, ts: u32) {
        self.reset();
        self.play_ts = ts;
        self.base = 0;
        self.state = State::Filling;
        self.filling_ticks = 0;
        self.stall_counted = 0;
    }

    /// Stores a packet's payload.  `marker` or a new source restarts the
    /// buffer when the timestamp does not continue the old stream.
    pub fn insert(&mut self, ts: u32, payload: &[u8]) {
        let units = payload.len() / self.unit_bytes;
        if units == 0 {
            return;
        }
        if self.state == State::Empty {
            self.restart(ts);
        }
        let delta = ts.wrapping_sub(self.play_ts) as i32;
        // A timestamp off the 10 ms grid or far from the playout point is a
        // new stream (the sender restarted or switched source).
        if delta % TS_PER_UNIT as i32 != 0 || delta >= (SLOTS as i32) * TS_PER_UNIT as i32 || delta < -((SLOTS as i32) * TS_PER_UNIT as i32) {
            self.stats.resyncs += 1;
            self.restart(ts);
        }
        let first = (ts.wrapping_sub(self.play_ts) as i32) / TS_PER_UNIT as i32;
        for (k, unit) in payload.chunks_exact(self.unit_bytes).enumerate() {
            let off = first + k as i32;
            if off < 0 {
                self.stats.late += 1;
                self.window_trouble = true;
                continue;
            }
            if off as usize >= SLOTS {
                self.stats.dropped += 1;
                continue;
            }
            let slot = (self.base + off as usize) % SLOTS;
            self.data[slot][..self.unit_bytes].copy_from_slice(unit);
            self.used[slot] = true;
            self.span = self.span.max(off as u32 + 1);
        }
    }

    fn advance(&mut self) -> bool {
        let had = std::mem::take(&mut self.used[self.base]);
        self.base = (self.base + 1) % SLOTS;
        self.play_ts = self.play_ts.wrapping_add(TS_PER_UNIT);
        self.span = self.span.saturating_sub(1);
        had
    }

    /// Takes the unit due now; call once per 10 ms.  `out` gets
    /// `unit_bytes` bytes on [`Pop::Frame`].
    ///
    /// When nothing at all is held at playout time the buffer stalls: it
    /// reports one lost unit and then waits (idle) until the target delay
    /// has built up again, so that a stream whose packets started coming
    /// later is not late forever.  A unit missing while later ones are held
    /// is plain loss and is skipped.
    pub fn pop(&mut self, out: &mut [u8]) -> Pop {
        if self.state == State::Empty {
            return Pop::Idle;
        }
        self.empty_ticks = if self.span == 0 { self.empty_ticks + 1 } else { 0 };
        if self.empty_ticks >= PAUSE_AFTER {
            // The sender paused (or is gone): start over with its next
            // packet, and do not count the stall as loss.
            self.state = State::Empty;
            self.stats.lost = self.stats.lost.saturating_sub(std::mem::take(&mut self.stall_counted));
            return Pop::Idle;
        }
        if self.state == State::Filling {
            self.filling_ticks += 1;
            if self.span == 0 || (self.span < self.target && self.filling_ticks < self.target) {
                return Pop::Idle;
            }
            self.state = State::Playing;
            self.stall_counted = 0;
        }

        self.window_min_span = self.window_min_span.min(self.span);
        self.window_pops += 1;
        if self.window_pops >= WINDOW {
            if self.window_trouble || self.wanted > self.target {
                self.target = (self.target + 1).min(self.max_target);
            } else if self.wanted < self.target {
                self.target -= 1;
            }
            // More held than needed through the whole window: skip a unit.
            if !self.window_trouble && self.window_min_span > self.target && self.span > 1 {
                self.advance();
                self.stats.dropped += 1;
            }
            self.window_pops = 0;
            self.window_min_span = u32::MAX;
            self.window_trouble = false;
        }

        if self.span == 0 {
            self.state = State::Filling;
            self.filling_ticks = 0;
            self.window_trouble = true;
            self.stats.lost += 1;
            self.stall_counted = 1;
            return Pop::Lost;
        }
        let slot = self.base;
        if self.advance() {
            out[..self.unit_bytes].copy_from_slice(&self.data[slot][..self.unit_bytes]);
            Pop::Frame
        } else {
            self.stats.lost += 1;
            self.window_trouble = true;
            Pop::Lost
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(first: u8) -> [u8; 160] {
        let mut p = [first; 160];
        p[80..].fill(first + 1);
        p
    }

    #[test]
    fn in_order_stream_plays_with_target_delay() {
        let mut jb = JitterBuffer::new(80, 20, 200);
        let mut out = [0u8; 80];
        assert_eq!(jb.pop(&mut out), Pop::Idle);
        let mut played = Vec::new();
        // 20 ms packets, one every other tick
        for tick in 0..100u32 {
            if tick % 2 == 0 {
                jb.insert(1000 + tick * 80, &pkt(tick as u8));
            }
            if jb.pop(&mut out) == Pop::Frame {
                played.push(out[0]);
            }
        }
        // every unit, in order, after a start-up delay of a few ticks
        assert!(played.len() >= 94, "{}", played.len());
        assert!(played.iter().enumerate().all(|(i, &v)| v == i as u8));
        assert_eq!(jb.stats().lost, 0);
        assert_eq!(jb.stats().late, 0);
    }

    #[test]
    fn reordering_and_loss() {
        let mut jb = JitterBuffer::new(80, 40, 200);
        let mut out = [0u8; 80];
        let unit = |v: u8| [v; 80];
        // units 0..6 arrive as 0, 2, 1, 3, (4 lost), 5
        for v in [0u8, 2, 1, 3, 5] {
            jb.insert(u32::from(v) * 80, &unit(v));
        }
        let mut seq = Vec::new();
        for _ in 0..6 {
            seq.push(match jb.pop(&mut out) {
                Pop::Frame => i32::from(out[0]),
                Pop::Lost => -1,
                Pop::Idle => -2,
            });
        }
        assert_eq!(seq, [0, 1, 2, 3, -1, 5]);
        assert_eq!(jb.stats().lost, 1);
        // a unit that arrives after its time is counted, not played
        jb.insert(4 * 80, &unit(4));
        assert_eq!(jb.stats().late, 1);
    }

    #[test]
    fn pause_and_timestamp_jump() {
        let mut jb = JitterBuffer::new(80, 20, 200);
        let mut out = [0u8; 80];
        for i in 0..6u32 {
            jb.insert(i * 80, &[i as u8; 80]);
        }
        let mut frames = 0;
        let mut lost = 0;
        for _ in 0..60 {
            match jb.pop(&mut out) {
                Pop::Frame => frames += 1,
                Pop::Lost => lost += 1,
                Pop::Idle => {}
            }
        }
        assert_eq!(frames, 6);
        // the silence after the last unit is a pause, not loss
        assert_eq!(lost, 1);
        assert_eq!(jb.stats().lost, 0);
        // the sender comes back with an unrelated timestamp
        jb.insert(0x8000_0000, &[9; 160]);
        jb.insert(0x8000_0000 + 160, &[9; 160]);
        let mut got = 0;
        for _ in 0..8 {
            if jb.pop(&mut out) == Pop::Frame {
                assert_eq!(out[0], 9);
                got += 1;
            }
        }
        assert_eq!(got, 4);
        // a jump while playing restarts as well
        jb.insert(0x1234_5601, &[7; 80]);
        assert_eq!(jb.stats().resyncs, 1);
    }

    #[test]
    fn adapts_to_jitter() {
        let mut jb = JitterBuffer::new(80, 20, 200);
        let mut out = [0u8; 80];
        let start = jb.stats().target;
        jb.set_jitter(400); // 50 ms
        let mut lost = 0;
        // bursty arrival: 60 ms worth every 6 ticks
        for tick in 0..3000u32 {
            if tick % 6 == 0 {
                jb.insert(tick * 80, &[1; 480]);
            }
            if tick > 1000 && jb.pop(&mut out) == Pop::Lost {
                lost += 1;
            } else if tick <= 1000 {
                jb.pop(&mut out);
            }
        }
        assert!(jb.stats().target > start, "target {}", jb.stats().target);
        assert_eq!(lost, 0);
        // calm again: the delay comes back down
        jb.set_jitter(0);
        for tick in 3000..9000u32 {
            jb.insert(tick * 80, &[1; 80]);
            jb.pop(&mut out);
        }
        assert!(jb.stats().target <= start, "{:?} depth {}", jb.stats(), jb.depth());
        assert!(jb.depth() <= start + 1, "depth {}", jb.depth());
    }
}
