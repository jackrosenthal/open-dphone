// SPDX-License-Identifier: GPL-2.0-or-later
//! Tone generation on the ARM (the DSP has no tone job; the stock firmware
//! generates tones host-side as well).  Integer only: a phase accumulator
//! over a sine table built at compile time.

/// Samples per second of generated tones (they go into the narrow band).
pub const TONE_RATE: u32 = 8_000;

const TABLE_BITS: u32 = 10;
const TABLE_LEN: usize = 1 << TABLE_BITS;

/// sin(x) for x in Q30 radians, |x| <= pi/2, result in Q30.
const fn sin_q30(x: i64) -> i64 {
    // Taylor series to x^11; error below 1e-7 on this range.
    let x2 = (x * x) >> 30;
    let mut term = x;
    let mut sum = x;
    let mut k = 1;
    while k <= 5 {
        term = -(((term * x2) >> 30) / ((2 * k) * (2 * k + 1)));
        sum += term;
        k += 1;
    }
    sum
}

const fn sine_table() -> [i16; TABLE_LEN] {
    const PI_Q30: i64 = 3_373_259_426; // pi * 2^30
    let mut t = [0i16; TABLE_LEN];
    let mut i = 0;
    while i < TABLE_LEN {
        // fold into the first quadrant
        let q = TABLE_LEN / 4;
        let (idx, neg) = if i < q {
            (i, false)
        } else if i < 2 * q {
            (2 * q - i, false)
        } else if i < 3 * q {
            (i - 2 * q, true)
        } else {
            (4 * q - i, true)
        };
        let x = PI_Q30 * idx as i64 / (2 * q as i64);
        let s = (sin_q30(x) * 32767 + (1 << 29)) >> 30;
        t[i] = if neg { -s as i16 } else { s as i16 };
        i += 1;
    }
    t
}

static SINE: [i16; TABLE_LEN] = sine_table();

/// The sine of `phase` (a full turn is 2^32), -32767..=32767: the table,
/// linearly interpolated.
#[inline]
pub fn sin_q15(phase: u32) -> i32 {
    let idx = (phase >> (32 - TABLE_BITS)) as usize;
    let frac = ((phase >> (32 - TABLE_BITS - 8)) & 0xff) as i32;
    let a = i32::from(SINE[idx]);
    let b = i32::from(SINE[(idx + 1) & (TABLE_LEN - 1)]);
    a + (((b - a) * frac) >> 8)
}

/// One sine oscillator.
#[derive(Clone, Debug)]
pub struct Osc {
    phase: u32,
    step: u32,
    /// Peak amplitude, 0..=32767.
    amp: i32,
}

impl Osc {
    pub fn new(freq_hz: u32, rate: u32, amp: i32) -> Self {
        Self { phase: 0, step: ((u64::from(freq_hz) << 32) / u64::from(rate)) as u32, amp }
    }

    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> i16 {
        let s = sin_q15(self.phase);
        self.phase = self.phase.wrapping_add(self.step);
        ((s * self.amp) >> 15) as i16
    }

    pub fn reset(&mut self) {
        self.phase = 0;
    }
}

/// Peak amplitude of a sine at `dbm0` (0 dBm0 is 3.14 dB below full scale).
/// Whole decibels from -40 to +3; integer arithmetic.
pub const fn amplitude_dbm0(dbm0: i32) -> i16 {
    // 0 dBm0 = 22_826; each dB is a factor 10^(1/20) = 1.12202 (Q16: 73_533)
    let mut a: i64 = 22_826 << 16;
    let mut d = dbm0;
    while d > 0 {
        a = (a * 73_533) >> 16;
        d -= 1;
    }
    while d < 0 {
        a = (a << 16) / 73_533;
        d += 1;
    }
    let v = a >> 16;
    if v > 32_767 { 32_767 } else { v as i16 }
}

/// A tone: up to two summed frequencies and an on/off cadence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToneSpec {
    /// Frequencies in Hz; 0 = unused.
    pub freqs: [u16; 2],
    /// Peak amplitude of each frequency.
    pub amps: [i16; 2],
    /// (on ms, off ms) segments; a segment with on = 0 ends the list.  An
    /// off time of 0 in the last segment means "on forever".
    pub cadence: [(u16, u16); 4],
    /// Repeat the cadence (a last segment with off = 0 then sounds
    /// forever); otherwise the tone ends after one pass.
    pub repeat: bool,
}

impl ToneSpec {
    pub const fn continuous(f1: u16, f2: u16, dbm0: i32) -> Self {
        let a = amplitude_dbm0(dbm0);
        Self { freqs: [f1, f2], amps: [a, if f2 == 0 { 0 } else { a }], cadence: [(1000, 0), (0, 0), (0, 0), (0, 0)], repeat: true }
    }

    pub const fn cadenced(f1: u16, f2: u16, dbm0: i32, on_ms: u16, off_ms: u16) -> Self {
        let mut t = Self::continuous(f1, f2, dbm0);
        t.cadence[0] = (on_ms, off_ms);
        t
    }

    pub const fn once(f1: u16, f2: u16, dbm0: i32, on_ms: u16) -> Self {
        let mut t = Self::cadenced(f1, f2, dbm0, on_ms, 0);
        t.repeat = false;
        t
    }

    // North American precise tone plan.
    pub const DIAL: Self = Self::continuous(350, 440, -13);
    pub const RINGBACK: Self = Self::cadenced(440, 480, -19, 2000, 4000);
    pub const BUSY: Self = Self::cadenced(480, 620, -24, 500, 500);
    pub const REORDER: Self = Self::cadenced(480, 620, -24, 250, 250);
    pub const CALL_WAITING: Self = Self::cadenced(440, 0, -13, 300, 10_000);
    /// Short confirmation beep.
    pub const BEEP: Self = Self::once(1000, 0, -16, 120);
    /// Stutter dial tone (e.g., message waiting): three bursts, then steady.
    pub const STUTTER_DIAL: Self = Self {
        freqs: [350, 440],
        amps: [amplitude_dbm0(-13), amplitude_dbm0(-13)],
        cadence: [(100, 100), (100, 100), (100, 100), (1000, 0)],
        repeat: true,
    };

    /// The DTMF pair of a key (`0`-`9`, `*`, `#`, `A`-`D`), at the given
    /// level per frequency, continuous until stopped.
    pub fn dtmf(key: char, dbm0: i32) -> Option<Self> {
        const ROWS: [u16; 4] = [697, 770, 852, 941];
        const COLS: [u16; 4] = [1209, 1336, 1477, 1633];
        const KEYS: [[char; 4]; 4] = [['1', '2', '3', 'A'], ['4', '5', '6', 'B'], ['7', '8', '9', 'C'], ['*', '0', '#', 'D']];
        let key = key.to_ascii_uppercase();
        for (r, row) in KEYS.iter().enumerate() {
            if let Some(c) = row.iter().position(|&k| k == key) {
                // the high group is sent 2 dB louder, as usual
                let mut t = Self::continuous(ROWS[r], COLS[c], dbm0);
                t.amps[1] = amplitude_dbm0(dbm0 + 2);
                return Some(t);
            }
        }
        None
    }
}

/// Ramp length in samples at tone edges (2 ms), against clicks.
const RAMP: i32 = 16;

/// Generates a [`ToneSpec`] at 8 kHz.
#[derive(Clone, Debug)]
pub struct ToneGen {
    spec: ToneSpec,
    osc: [Osc; 2],
    seg: usize,
    /// Samples into the current segment.
    pos: u32,
    /// Envelope 0..=RAMP.
    env: i32,
    done: bool,
}

impl ToneGen {
    pub fn new(spec: ToneSpec) -> Self {
        Self {
            osc: [
                Osc::new(u32::from(spec.freqs[0]), TONE_RATE, i32::from(spec.amps[0])),
                Osc::new(u32::from(spec.freqs[1]), TONE_RATE, i32::from(spec.amps[1])),
            ],
            spec,
            seg: 0,
            pos: 0,
            env: 0,
            done: spec.cadence[0].0 == 0,
        }
    }

    pub fn spec(&self) -> &ToneSpec {
        &self.spec
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Adds the tone to `out` (8 kHz samples).  Returns false once a
    /// non-repeating tone has ended.
    pub fn mix_into(&mut self, out: &mut [i16]) -> bool {
        if self.done {
            return false;
        }
        for o in out.iter_mut() {
            let (on_ms, off_ms) = self.spec.cadence[self.seg];
            let on = u32::from(on_ms) * (TONE_RATE / 1000);
            let off = u32::from(off_ms) * (TONE_RATE / 1000);
            let forever = off == 0 && self.last_segment() && self.spec.repeat;
            let sounding = forever || self.pos < on;
            // ramp down just before the end of the on time
            let target = if sounding && (forever || on - self.pos > RAMP as u32) { RAMP } else { 0 };
            self.env += (target - self.env).signum();
            if self.env > 0 {
                let mut s = i32::from(self.osc[0].next());
                if self.spec.freqs[1] != 0 {
                    s += i32::from(self.osc[1].next());
                }
                *o = crate::bands::sat16(i32::from(*o) + s * self.env / RAMP);
            }
            if !forever {
                self.pos += 1;
                if self.pos >= on + off {
                    self.pos = 0;
                    self.env = 0;
                    self.osc[0].reset();
                    self.osc[1].reset();
                    if self.last_segment() {
                        if !self.spec.repeat {
                            self.done = true;
                            return false;
                        }
                        self.seg = 0;
                    } else {
                        self.seg += 1;
                    }
                }
            }
        }
        true
    }

    fn last_segment(&self) -> bool {
        self.seg == 3 || self.spec.cadence[self.seg + 1].0 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goertzel(x: &[i16], f: u32, rate: u32) -> f64 {
        let w = 2.0 * std::f64::consts::PI * f64::from(f) / f64::from(rate);
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0, 0.0);
        for &v in x {
            let s0 = f64::from(v) + c * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        ((s1 * s1 + s2 * s2 - c * s1 * s2).sqrt()) * 2.0 / x.len() as f64
    }

    #[test]
    fn sine_table_is_a_sine() {
        for (i, &v) in SINE.iter().enumerate() {
            let want = (2.0 * std::f64::consts::PI * i as f64 / TABLE_LEN as f64).sin() * 32767.0;
            assert!((f64::from(v) - want).abs() <= 1.0, "{i}: {v} vs {want}");
        }
    }

    #[test]
    fn levels() {
        assert_eq!(amplitude_dbm0(0), 22_826);
        assert!((i32::from(amplitude_dbm0(-13)) - 5110).abs() < 10);
        assert!((i32::from(amplitude_dbm0(-20)) - 2283).abs() < 5);
        assert_eq!(amplitude_dbm0(10), 32_767);
    }

    #[test]
    fn dial_tone_has_its_two_frequencies() {
        let mut g = ToneGen::new(ToneSpec::DIAL);
        let mut buf = vec![0i16; 8000];
        assert!(g.mix_into(&mut buf));
        let a = f64::from(amplitude_dbm0(-13));
        assert!((goertzel(&buf, 350, 8000) - a).abs() < a * 0.02);
        assert!((goertzel(&buf, 440, 8000) - a).abs() < a * 0.02);
    }

    #[test]
    fn cadence_and_end() {
        // busy: 0.5 s on, 0.5 s off
        let mut g = ToneGen::new(ToneSpec::BUSY);
        let mut buf = vec![0i16; 16000];
        g.mix_into(&mut buf);
        assert!(buf[400..3900].iter().any(|&s| s != 0));
        assert!(buf[4000..8000].iter().all(|&s| s == 0));
        assert!(buf[8100..11900].iter().any(|&s| s != 0));
        // a one-shot ends
        let mut g = ToneGen::new(ToneSpec::BEEP);
        let mut buf = vec![0i16; 8000];
        let mut alive = true;
        for chunk in buf.chunks_mut(80) {
            alive = g.mix_into(chunk);
            if !alive {
                break;
            }
        }
        assert!(!alive && g.is_done());
        assert!(buf[..960].iter().any(|&s| s != 0));
        assert!(buf[960..].iter().all(|&s| s == 0));
        // stutter dial ends steady
        let mut g = ToneGen::new(ToneSpec::STUTTER_DIAL);
        let mut buf = vec![0i16; 24000];
        g.mix_into(&mut buf);
        assert!(buf[820..1580].iter().all(|&s| s == 0));
        assert!(buf[16000..].iter().filter(|&&s| s != 0).count() > 7000);
    }

    #[test]
    fn dtmf_pairs() {
        let t = ToneSpec::dtmf('5', -10).unwrap();
        assert_eq!(t.freqs, [770, 1336]);
        assert_eq!(ToneSpec::dtmf('#', -10).unwrap().freqs, [941, 1477]);
        assert_eq!(ToneSpec::dtmf('d', -10).unwrap().freqs, [941, 1633]);
        assert!(ToneSpec::dtmf('x', -10).is_none());
    }
}
