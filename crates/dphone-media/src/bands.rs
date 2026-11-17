// SPDX-License-Identifier: GPL-2.0-or-later
//! Band frames: the form in which 10 ms of audio passes through the
//! cancellers, and a two-band filter bank for the old family.
//!
//! The DSP's canceller and filter-bank jobs carry audio as consecutive
//! buffers.  The captured job bodies show what they hold (energy per
//! frequency range, over thousands of frames from a D60 and a D40):
//!
//! - buffer 0, 80 samples at 8 kHz: the signal below 4 kHz;
//! - buffer 1, 160 samples at 16 kHz: only the 4 to 8 kHz part (no energy
//!   below about 3 kHz), not decimated and not shifted;
//! - buffer 2, 320 samples at 32 kHz (new family): only the part above
//!   8 kHz.
//!
//! So a wider signal is the narrower one interpolated plus the next buffer.
//! On the new family the DSP's BAND client converts between PCM and these
//! buffers.  The old family has no such client; its vendor modules do the
//! conversion on the ARM, with filters that were not recovered.  The filter
//! bank here has the same structure (a half-band low-pass for the lower
//! buffer, the remainder in the upper one) and reconstructs exactly when
//! nothing touched the bands; it is used for both directions, so it only
//! has to agree with itself.

/// Samples per 10 ms in each band buffer.
pub const NB: usize = 80;
pub const WB: usize = 160;
pub const SWB: usize = 320;

/// 10 ms of audio as band buffers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BandFrame {
    /// Below 4 kHz, at 8 kHz.
    pub nb: [i16; NB],
    /// 4 to 8 kHz, at 16 kHz.
    pub hi1: [i16; WB],
    /// Above 8 kHz, at 32 kHz (new family; zero otherwise).
    pub hi2: [i16; SWB],
}

impl Default for BandFrame {
    fn default() -> Self {
        Self::ZERO
    }
}

#[inline]
pub fn sat16(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

impl BandFrame {
    pub const ZERO: BandFrame = BandFrame { nb: [0; NB], hi1: [0; WB], hi2: [0; SWB] };

    pub fn clear(&mut self) {
        *self = Self::ZERO;
    }

    /// Adds `other`, saturating.  Mixing is linear, so it can be done per
    /// buffer.
    pub fn mix(&mut self, other: &BandFrame) {
        mix_into(&mut self.nb, &other.nb);
        mix_into(&mut self.hi1, &other.hi1);
        mix_into(&mut self.hi2, &other.hi2);
    }

    pub fn is_silent(&self) -> bool {
        self.nb.iter().all(|&s| s == 0) && self.hi1.iter().all(|&s| s == 0) && self.hi2.iter().all(|&s| s == 0)
    }

    /// The job body of a canceller frame: the first `bytes` of the three
    /// buffers laid end to end (480 bytes: two buffers; 1120: three).
    pub fn to_bytes(&self, out: &mut [u8]) {
        let mut it = self.nb.iter().chain(&self.hi1).chain(&self.hi2);
        for pair in out.as_chunks_mut::<2>().0 {
            pair.copy_from_slice(&it.next().copied().unwrap_or(0).to_le_bytes());
        }
    }

    /// The inverse of [`Self::to_bytes`]; buffers the bytes do not reach
    /// are left alone.
    pub fn from_bytes(&mut self, bytes: &[u8]) {
        let it = self.nb.iter_mut().chain(&mut self.hi1).chain(&mut self.hi2);
        for (s, pair) in it.zip(bytes.as_chunks::<2>().0) {
            *s = i16::from_le_bytes([pair[0], pair[1]]);
        }
    }
}

pub fn mix_into(dst: &mut [i16], src: &[i16]) {
    for (d, &s) in dst.iter_mut().zip(src) {
        *d = sat16(i32::from(*d) + i32::from(s));
    }
}

/// Applies a gain in Q12 (4096 = unity), saturating.
pub fn scale(buf: &mut [i16], gain_q12: i32) {
    if gain_q12 == 4096 {
        return;
    }
    for s in buf {
        *s = sat16((i32::from(*s) * gain_q12) >> 12);
    }
}

/// Half-band low-pass, 47 taps, Q15.  Tap 0 is 0.5; even taps are zero;
/// these are the odd taps 1, 3, ..., 23 (the filter is symmetric).  Flat to
/// 3.4 kHz and 45 dB down at 4.6 kHz for a 16 kHz signal.
const HB_ODD: [i32; 12] = [10378, -3318, 1831, -1150, 750, -489, 311, -189, 107, -55, 23, -7];
const K: usize = 23;
/// Delay of split followed by combine, in samples of the wide signal.
pub const BANK_DELAY: usize = 2 * K;

/// Interpolation by two with the half-band filter, with its own history.
#[derive(Clone, Debug)]
struct Interp2 {
    /// The last 24 narrow samples before the current block.
    hist: [i16; K + 1],
}

impl Interp2 {
    const fn new() -> Self {
        Self { hist: [0; K + 1] }
    }

    /// `out[n]` for n in 0..2 * low.len(): the interpolated signal, delayed
    /// by K wide samples.
    fn run(&mut self, low: &[i16], mut emit: impl FnMut(usize, i32)) {
        const H: usize = K + 1;
        debug_assert!(low.len() >= H);
        // low sample j (may be negative: history)
        let at = |hist: &[i16; H], j: isize| -> i32 {
            if j < 0 { i32::from(hist[(H as isize + j) as usize]) } else { i32::from(low[j as usize]) }
        };
        for m in 0..low.len() as isize {
            // Even wide index n = 2m: odd taps around the center between
            // low samples.  n - K = 2m - K; taps at odd offsets d = 2i + 1
            // reach low samples (2m - K -/+ d) / 2.
            let c = 2 * m - K as isize;
            let mut acc: i32 = 0;
            for (i, &h) in HB_ODD.iter().enumerate() {
                let d = 2 * i as isize + 1;
                acc += h * (at(&self.hist, (c - d) / 2) + at(&self.hist, (c + d) / 2));
            }
            // gain 2 for interpolation, Q15
            emit(2 * m as usize, (acc + (1 << 13)) >> 14);
            // Odd wide index n = 2m + 1: n - K is even, only the center tap
            // (0.5 * 2 = 1) contributes.
            emit(2 * m as usize + 1, at(&self.hist, (2 * m + 1 - K as isize) / 2));
        }
        let n = low.len();
        self.hist.copy_from_slice(&low[n - H..]);
    }
}

/// Splits a signal into the half-rate low band and the full-rate remainder.
#[derive(Clone, Debug)]
pub struct Split2 {
    /// The last 2K wide samples before the current block.
    hist: [i16; 2 * K],
    interp: Interp2,
}

impl Default for Split2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Split2 {
    pub const fn new() -> Self {
        Self { hist: [0; 2 * K], interp: Interp2::new() }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// `x` has 2N samples; `low` gets N, `high` gets 2N.  N is at least 24.
    pub fn run(&mut self, x: &[i16], low: &mut [i16], high: &mut [i16]) {
        let n = low.len();
        debug_assert!(x.len() == 2 * n && high.len() == 2 * n);
        const HL: isize = 2 * K as isize;
        let hist = self.hist;
        let at = |i: isize| -> i32 { if i < 0 { i32::from(hist[(HL + i) as usize]) } else { i32::from(x[i as usize]) } };
        // low[m] = sum h[k] x[2m - K - k], k = -K..K
        for (m, l) in low.iter_mut().enumerate() {
            let c = 2 * m as isize - K as isize;
            let mut acc: i32 = at(c) << 14;
            for (i, &h) in HB_ODD.iter().enumerate() {
                let d = 2 * i as isize + 1;
                acc += h * (at(c - d) + at(c + d));
            }
            *l = sat16((acc + (1 << 14)) >> 15);
        }
        // high[n] = x[n - 2K] - interp(low)[n]
        self.interp.run(low, |i, up| high[i] = sat16(at(i as isize - HL) - up));
        self.hist.copy_from_slice(&x[2 * n - 2 * K..]);
    }

    /// Only the low band, for where the remainder is not wanted.
    pub fn low_only(&mut self, x: &[i16], low: &mut [i16]) {
        let mut scratch = [0i16; SWB];
        self.run(x, low, &mut scratch[..x.len()]);
    }
}

/// The inverse of [`Split2`]: low band interpolated, plus the remainder.
#[derive(Clone, Debug)]
pub struct Combine2 {
    interp: Interp2,
}

impl Default for Combine2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Combine2 {
    pub const fn new() -> Self {
        Self { interp: Interp2::new() }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// `low` has N samples, `high` and `out` 2N.
    pub fn run(&mut self, low: &[i16], high: &[i16], out: &mut [i16]) {
        debug_assert!(high.len() == 2 * low.len() && out.len() == high.len());
        self.interp.run(low, |i, up| out[i] = sat16(up + i32::from(high[i])));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integer sine: amplitude `a`, `f` Hz at `rate`.
    pub(crate) fn sine(n: usize, f: u32, rate: u32, a: i32) -> Vec<i16> {
        let mut osc = crate::tone::Osc::new(f, rate, a);
        (0..n).map(|_| osc.next()).collect()
    }

    pub(crate) fn rms(x: &[i16]) -> i32 {
        let s: i64 = x.iter().map(|&v| i64::from(v) * i64::from(v)).sum();
        (((s / x.len().max(1) as i64) as f64).sqrt()) as i32
    }

    #[test]
    fn split_combine_reconstructs_with_delay() {
        // a mix of a low and a high tone plus a ramp, in 10 ms blocks
        let n = 16 * WB;
        let a = sine(n, 700, 16_000, 6000);
        let b = sine(n, 6100, 16_000, 3000);
        let x: Vec<i16> = (0..n).map(|i| a[i] + b[i] + (i % 37) as i16 * 20).collect();
        let (mut split, mut comb) = (Split2::new(), Combine2::new());
        let mut y = vec![0i16; n];
        for (xi, yi) in x.chunks(WB).zip(y.chunks_mut(WB)) {
            let (mut low, mut high) = ([0i16; NB], [0i16; WB]);
            split.run(xi, &mut low, &mut high);
            comb.run(&low, &high, yi);
        }
        for i in BANK_DELAY..n {
            let d = i32::from(y[i]) - i32::from(x[i - BANK_DELAY]);
            assert!(d.abs() <= 1, "sample {i}: {} vs {}", y[i], x[i - BANK_DELAY]);
        }
    }

    #[test]
    fn bands_hold_the_right_frequencies() {
        let n = 20 * WB;
        for (f, in_low) in [(500, true), (2500, true), (3300, true), (5000, false), (7000, false)] {
            let x = sine(n, f, 16_000, 8000);
            let mut split = Split2::new();
            let (mut lows, mut highs) = (Vec::new(), Vec::new());
            for xi in x.chunks(WB) {
                let (mut low, mut high) = ([0i16; NB], [0i16; WB]);
                split.run(xi, &mut low, &mut high);
                lows.extend_from_slice(&low);
                highs.extend_from_slice(&high);
            }
            let (l, h) = (rms(&lows[NB..]), rms(&highs[WB..]));
            if in_low {
                assert!(l > 5000 && h < 120, "{f} Hz: low {l}, high {h}");
            } else {
                assert!(l < 60 && h > 5000, "{f} Hz: low {l}, high {h}");
            }
        }
    }

    #[test]
    fn frame_bytes_round_trip() {
        let mut f = BandFrame::ZERO;
        for (i, s) in f.nb.iter_mut().enumerate() {
            *s = i as i16;
        }
        f.hi1[0] = -5;
        f.hi2[319] = 77;
        let mut b = [0u8; 1120];
        f.to_bytes(&mut b);
        assert_eq!(&b[2..4], &[1, 0]);
        let mut g = BandFrame::ZERO;
        g.from_bytes(&b);
        assert_eq!(f, g);
        let mut two = [0u8; 480];
        f.to_bytes(&mut two);
        let mut g = BandFrame::ZERO;
        g.from_bytes(&two);
        assert_eq!(g.hi1[0], -5);
    }
}
