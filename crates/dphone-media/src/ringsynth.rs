// SPDX-License-Identifier: GPL-2.0-or-later
//! The synthesizer of the built-in ringtones: a few voices, each a struck
//! or held note made of up to four sine partials with their own exponential
//! decays, the first of which a modulator can phase-modulate (two-operator
//! FM).  A voice has an attack ramp, a release when its gate closes, and,
//! for a bell that a clapper keeps striking, a repeated strike.  Notes ring
//! over one another.  Integer only (the target has no floating point), and
//! nothing is allocated.
//!
//! A phone's loudspeaker gives little below about 450 Hz, so the scores
//! keep their fundamentals between about 500 Hz and 1600 Hz, and the
//! partials above 2.5 kHz are weak and die quickly.

use crate::tone::sin_q15;

/// Samples per second.
pub const RATE: u32 = 16_000;
const PER_MS: u32 = RATE / 1000;

/// Voices that can sound at once.  The scores never need more (a test
/// holds them to it); a note that finds no free voice is not played.
pub const VOICES: usize = 8;
const PARTS: usize = 4;

/// Unity of a voice's gain and of an envelope's scale, Q30.
const ONE: i32 = 1 << 30;
/// An envelope below this is an amplitude below one: the partial is silent.
const ENV_MIN: i32 = 1 << 15;
/// Partials at or above this frequency are left out (the Nyquist frequency
/// is 8 kHz, and nothing up there is pleasant).
const MAX_HZ: u32 = 7000;

/// One partial of a timbre.
#[derive(Clone, Copy, Debug)]
pub struct Partial {
    /// Frequency as a multiple of the note's, Q10.
    pub ratio: u16,
    /// Amplitude relative to the note's level, Q15.
    pub amp: u16,
    /// Time in milliseconds in which the partial falls to 1/e; 0 = it holds.
    pub tau_ms: u16,
}

/// A modulator on the first partial's phase.
#[derive(Clone, Copy, Debug)]
pub struct Fm {
    /// Frequency as a multiple of the note's, Q10.
    pub ratio: u16,
    /// Peak phase deviation in radians, Q8; at most 3 radians.
    pub index: u16,
    /// Time in milliseconds in which the deviation falls to 1/e.
    pub tau_ms: u16,
}

/// A clapper that strikes again and again while the gate is open: at each
/// strike the voice's gain rises to full, then falls toward `floor`.
#[derive(Clone, Copy, Debug)]
pub struct Strike {
    pub period_ms: u16,
    /// Samples the gain takes to rise.
    pub rise: u16,
    /// What the gain falls toward, Q15.
    pub floor: u16,
    /// Time in milliseconds in which it covers 1/e of the way.
    pub tau_ms: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct Timbre {
    pub partials: &'static [Partial],
    pub fm: Option<Fm>,
    /// Samples the gain takes to rise from nothing when the note starts.
    pub attack: u16,
    /// Time constant in milliseconds of every partial once the gate closes.
    pub release_ms: u16,
    pub strike: Option<Strike>,
}

#[derive(Clone, Copy, Debug)]
pub struct Note {
    /// When the note starts, in milliseconds from the start of the cycle.
    pub at_ms: u16,
    pub hz: u16,
    /// Loudness, percent of the score's amplitude.
    pub vel: u8,
    /// How long the note rings before it is damped; 0 = it is never damped.
    pub gate_ms: u16,
    pub timbre: &'static Timbre,
}

/// One ringtone: notes in order of their start, and the length of the
/// cycle (the phrase and the pause after it).
#[derive(Clone, Copy, Debug)]
pub struct Score {
    pub notes: &'static [Note],
    pub cycle_ms: u16,
    /// Peak amplitude of a partial of relative amplitude 1 in a note of
    /// velocity 100.
    pub amp: i16,
}

/// The factor, Q31, that an envelope is multiplied by every sample to fall
/// to 1/e in `tau_ms`.
const fn decay_q31(tau_ms: u16) -> i32 {
    if tau_ms == 0 {
        return i32::MAX;
    }
    // exp(-x) for x = 1 / (tau in samples), at most 1/16: three terms of
    // the series leave an error below 1e-6.
    let one: i64 = 1 << 31;
    let x = one / (tau_ms as i64 * PER_MS as i64);
    let x2 = (x * x) >> 31;
    let x3 = (x2 * x) >> 31;
    (one - x + x2 / 2 - x3 / 6) as i32
}

#[inline]
fn decayed(env: i32, decay: i32) -> i32 {
    ((i64::from(env) * i64::from(decay)) >> 31) as i32
}

#[derive(Clone, Copy, Debug)]
struct Part {
    phase: u32,
    step: u32,
    /// Amplitude, Q15, times 2^15.
    env: i32,
    decay: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gain {
    /// Rising by `rise_step` a sample to full.
    Rising,
    /// Falling toward the strike's floor.
    Falling,
    Holding,
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    live: bool,
    parts: [Part; PARTS],
    fm_phase: u32,
    /// 0 = no modulator.
    fm_step: u32,
    /// Phase deviation per unit of the modulator's sine, times 2^14.
    fm_env: i32,
    fm_decay: i32,
    /// Q30.
    gain: i32,
    mode: Gain,
    rise_step: i32,
    /// Samples until the gate closes; `u32::MAX` = never.
    gate_left: u32,
    release: i32,
    /// Samples between strikes; 0 = the voice is struck once.
    strike_period: u32,
    strike_left: u32,
    strike_rise_step: i32,
    floor: i32,
    fall: i32,
}

impl Voice {
    const IDLE: Voice = Voice {
        live: false,
        parts: [Part { phase: 0, step: 0, env: 0, decay: 0 }; PARTS],
        fm_phase: 0,
        fm_step: 0,
        fm_env: 0,
        fm_decay: 0,
        gain: 0,
        mode: Gain::Holding,
        rise_step: 0,
        gate_left: u32::MAX,
        release: 0,
        strike_period: 0,
        strike_left: u32::MAX,
        strike_rise_step: 0,
        floor: 0,
        fall: 0,
    };

    fn start(note: &Note, amp: i16) -> Voice {
        let t = note.timbre;
        let level = i32::from(amp) * i32::from(note.vel) / 100;
        let step = |ratio: u16| -> Option<u32> {
            let hz_q10 = u64::from(note.hz) * u64::from(ratio);
            (hz_q10 < u64::from(MAX_HZ) << 10).then(|| ((hz_q10 << 22) / u64::from(RATE)) as u32)
        };
        let mut v = Voice::IDLE;
        v.live = true;
        for (part, spec) in v.parts.iter_mut().zip(t.partials) {
            if let Some(step) = step(spec.ratio) {
                *part = Part { phase: 0, step, env: level * i32::from(spec.amp.min(1 << 15)), decay: decay_q31(spec.tau_ms) };
            }
        }
        if let Some(fm) = t.fm
            && let Some(step) = step(fm.ratio)
        {
            // `index` radians are index / (2 pi) turns of 2^32; the sine is
            // Q15, so the factor is index / (2 pi) * 2^17, and it is kept
            // times 2^14.  A softer note is also a duller one.
            let k = (i64::from(fm.index.min(3 << 8)) * 20_861) >> 8;
            v.fm_step = step;
            v.fm_env = ((k * i64::from(note.vel.min(100)) / 100) << 14) as i32;
            v.fm_decay = decay_q31(fm.tau_ms);
        }
        v.mode = Gain::Rising;
        v.rise_step = ONE / i32::from(t.attack.max(1));
        if note.gate_ms != 0 {
            v.gate_left = u32::from(note.gate_ms) * PER_MS;
        }
        v.release = decay_q31(t.release_ms.max(1));
        if let Some(s) = t.strike {
            v.strike_period = u32::from(s.period_ms.max(1)) * PER_MS;
            v.strike_left = v.strike_period;
            v.strike_rise_step = ONE / i32::from(s.rise.max(1));
            v.floor = i32::from(s.floor.min(1 << 15)) << 15;
            v.fall = decay_q31(s.tau_ms.max(1));
        }
        v
    }

    /// Adds the voice's next `acc.len()` samples to `acc`.
    fn render(&mut self, acc: &mut [i32]) {
        let mut off = 0;
        while off < acc.len() {
            let n = (acc.len() - off).min(self.gate_left as usize).min(self.strike_left as usize);
            self.run(&mut acc[off..off + n]);
            off += n;
            if self.gate_left != u32::MAX {
                self.gate_left -= n as u32;
                if self.gate_left == 0 {
                    // Damped: everything dies away, and the clapper stops.
                    self.gate_left = u32::MAX;
                    self.strike_left = u32::MAX;
                    for p in &mut self.parts {
                        p.decay = p.decay.min(self.release);
                    }
                    if self.mode == Gain::Falling {
                        self.mode = Gain::Holding;
                    }
                }
            }
            if self.strike_left != u32::MAX {
                self.strike_left -= n as u32;
                if self.strike_left == 0 {
                    self.strike_left = self.strike_period;
                    self.mode = Gain::Rising;
                    self.rise_step = self.strike_rise_step;
                }
            }
        }
        self.live = self.parts.iter().any(|p| p.env >= ENV_MIN);
    }

    /// `acc.len()` samples with no event among them.
    fn run(&mut self, acc: &mut [i32]) {
        let mut sum = [0i32; crate::bands::WB];
        let sum = &mut sum[..acc.len()];
        for (k, part) in self.parts.iter_mut().enumerate() {
            let Part { mut phase, step, mut env, decay } = *part;
            if env < ENV_MIN {
                continue;
            }
            if k == 0 && self.fm_step != 0 {
                let (mut fm_phase, mut fm_env) = (self.fm_phase, self.fm_env);
                for s in sum.iter_mut() {
                    let deviation = sin_q15(fm_phase) * (fm_env >> 14);
                    *s += (sin_q15(phase.wrapping_add(deviation as u32)) * (env >> 15)) >> 15;
                    phase = phase.wrapping_add(step);
                    fm_phase = fm_phase.wrapping_add(self.fm_step);
                    env = decayed(env, decay);
                    fm_env = decayed(fm_env, self.fm_decay);
                }
                (self.fm_phase, self.fm_env) = (fm_phase, fm_env);
            } else {
                for s in sum.iter_mut() {
                    *s += (sin_q15(phase) * (env >> 15)) >> 15;
                    phase = phase.wrapping_add(step);
                    env = decayed(env, decay);
                }
            }
            (part.phase, part.env) = (phase, env);
        }
        if self.mode == Gain::Holding && self.gain == ONE {
            for (a, s) in acc.iter_mut().zip(sum.iter()) {
                *a += *s;
            }
            return;
        }
        let mut gain = self.gain;
        for (a, s) in acc.iter_mut().zip(sum.iter()) {
            match self.mode {
                Gain::Rising => {
                    gain += self.rise_step;
                    if gain >= ONE {
                        gain = ONE;
                        self.mode = if self.strike_left != u32::MAX { Gain::Falling } else { Gain::Holding };
                    }
                }
                Gain::Falling => gain = self.floor + decayed(gain - self.floor, self.fall),
                Gain::Holding => {}
            }
            *a += (*s * (gain >> 16)) >> 14;
        }
        self.gain = gain;
    }
}

/// Plays a score around and around.
#[derive(Clone, Debug)]
pub struct Synth {
    score: &'static Score,
    /// Samples into the cycle.
    pos: u32,
    /// The next note to start.
    next: usize,
    voices: [Voice; VOICES],
    /// Notes that found no free voice, and the most voices ever sounding.
    dropped: u32,
    most: usize,
}

impl Synth {
    pub fn new(score: &'static Score) -> Synth {
        Synth { score, pos: 0, next: 0, voices: [Voice::IDLE; VOICES], dropped: 0, most: 0 }
    }

    /// Notes that were not played for want of a voice, and the most voices
    /// that have sounded at once.
    pub fn load(&self) -> (u32, usize) {
        (self.dropped, self.most)
    }

    /// Adds the next `out.len()` samples (at most 10 ms) to `out`.
    pub fn mix_into(&mut self, out: &mut [i32]) {
        let cycle = u32::from(self.score.cycle_ms) * PER_MS;
        let mut off = 0;
        while off < out.len() {
            let notes = self.score.notes;
            while let Some(note) = notes.get(self.next)
                && u32::from(note.at_ms) * PER_MS <= self.pos
            {
                match self.voices.iter_mut().find(|v| !v.live) {
                    Some(v) => *v = Voice::start(note, self.score.amp),
                    None => self.dropped += 1,
                }
                self.next += 1;
            }
            self.most = self.most.max(self.voices.iter().filter(|v| v.live).count());
            let until = notes.get(self.next).map_or(cycle, |n| (u32::from(n.at_ms) * PER_MS).min(cycle)) - self.pos;
            let n = (out.len() - off).min(until as usize);
            for v in self.voices.iter_mut().filter(|v| v.live) {
                v.render(&mut out[off..off + n]);
            }
            off += n;
            self.pos += n as u32;
            if self.pos >= cycle {
                self.pos = 0;
                self.next = 0;
            }
        }
    }
}

// ---- timbres ---------------------------------------------------------------

const fn p(ratio: f32, amp: f32, tau_ms: u16) -> Partial {
    Partial { ratio: (ratio * 1024.0 + 0.5) as u16, amp: (amp * 32768.0) as u16, tau_ms }
}

/// A wooden bar under a soft mallet: the fundamental, the bar's tuned
/// fourth harmonic as a short knock, and nothing else.
static BAR: Timbre = Timbre { partials: &[p(1.0, 1.0, 260), p(4.0, 0.22, 40)], fm: None, attack: 48, release_ms: 70, strike: None };

/// A small hand bell: near-harmonic partials, the upper ones a little
/// sharp and shorter-lived, and a second fundamental just off the first
/// for a slow shimmer.
static BELL: Timbre = Timbre {
    partials: &[p(1.0, 0.8, 520), p(1.003, 0.3, 520), p(2.005, 0.34, 300), p(3.01, 0.12, 150)],
    fm: None,
    attack: 32,
    release_ms: 260,
    strike: None,
};

/// One gong of a telephone's ringer, struck twenty times a second.
static GONG: Timbre = Timbre {
    partials: &[p(1.0, 0.7, 0), p(1.006, 0.3, 0), p(2.02, 0.24, 0), p(2.83, 0.05, 0)],
    fm: None,
    attack: 24,
    release_ms: 170,
    strike: Some(Strike { period_ms: 50, rise: 32, floor: 14_000, tau_ms: 22 }),
};

/// An electric piano: a tine whose brightness (the modulation) dies much
/// sooner than its tone.
static TINE: Timbre = Timbre {
    partials: &[p(1.0, 1.0, 480)],
    fm: Some(Fm { ratio: 1024, index: 360, tau_ms: 120 }),
    attack: 32,
    release_ms: 90,
    strike: None,
};

/// A soft, nearly pure tone that swells in over 40 ms.
static SOFT: Timbre = Timbre { partials: &[p(1.0, 1.0, 450), p(2.0, 0.1, 250), p(3.0, 0.03, 150)], fm: None, attack: 640, release_ms: 220, strike: None };

/// A rounded electronic beep: it holds while the gate is open.
static BEEP: Timbre = Timbre { partials: &[p(1.0, 1.0, 600), p(2.0, 0.16, 300), p(3.0, 0.05, 200)], fm: None, attack: 160, release_ms: 25, strike: None };

// ---- scores ----------------------------------------------------------------

const fn note(at_ms: u16, hz: u16, vel: u8, gate_ms: u16, timbre: &'static Timbre) -> Note {
    Note { at_ms, hz, vel, gate_ms, timbre }
}

// Equal-tempered pitches, in Hz.
const C5: u16 = 523;
const E5: u16 = 659;
const G5: u16 = 784;
const B5: u16 = 988;
const C6: u16 = 1047;
const D6: u16 = 1175;
const E6: u16 = 1319;

/// A two-bar phrase on a marimba in G, ending on an open fifth.
pub static MARIMBA: Score = Score {
    notes: &[
        note(0, B5, 85, 520, &BAR),
        note(170, D6, 95, 520, &BAR),
        note(340, G5, 75, 520, &BAR),
        note(510, B5, 90, 600, &BAR),
        note(850, E6, 100, 520, &BAR),
        note(1020, D6, 85, 520, &BAR),
        note(1190, B5, 80, 600, &BAR),
        note(1530, G5, 90, 1100, &BAR),
        note(1530, D6, 70, 1100, &BAR),
    ],
    cycle_ms: 5000,
    amp: 7000,
};

/// The ringer of a telephone with a bell: two gongs a major third apart,
/// the clapper going from one to the other, two seconds on and four off.
pub static CLASSIC: Score = Score { notes: &[note(0, 880, 100, 2000, &GONG), note(25, 1109, 90, 2000, &GONG)], cycle_ms: 6000, amp: 7400 };

/// Three bell notes falling through a major chord, left to ring together.
pub static CHIME: Score =
    Score { notes: &[note(0, E6, 95, 1700, &BELL), note(400, C6, 95, 1500, &BELL), note(800, G5, 100, 1500, &BELL)], cycle_ms: 5000, amp: 8200 };

/// Two rounded beeps, the second a fourth higher.
pub static PULSE: Score = Score { notes: &[note(0, B5, 100, 130, &BEEP), note(250, E6, 90, 130, &BEEP)], cycle_ms: 3000, amp: 8800 };

/// An electric piano running up a major chord.
pub static RISE: Score = Score {
    notes: &[note(0, C5, 85, 420, &TINE), note(120, E5, 88, 420, &TINE), note(240, G5, 92, 420, &TINE), note(360, C6, 96, 420, &TINE), note(480, E6, 100, 1000, &TINE)],
    cycle_ms: 4500,
    amp: 6000,
};

/// Two soft notes, the second a minor third down: for a quiet office.
pub static MELLOW: Score = Score { notes: &[note(0, G5, 100, 800, &SOFT), note(480, E5, 92, 1000, &SOFT)], cycle_ms: 5000, amp: 7400 };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_factors() {
        // A time constant of 100 ms: e^(-1) after 1600 samples.
        let d = decay_q31(100);
        let mut env = ONE;
        for _ in 0..1600 {
            env = decayed(env, d);
        }
        let want = (f64::from(ONE) / std::f64::consts::E) as i32;
        assert!((env - want).abs() < want / 1000, "{env} {want}");
        assert_eq!(decay_q31(0), i32::MAX);
    }

    /// Every note of every score, alone: it comes in under its attack
    /// ramp, dies away smoothly once damped, and ends in exact silence.
    #[test]
    fn notes_start_and_end_without_a_click() {
        for score in [&MARIMBA, &CLASSIC, &CHIME, &PULSE, &RISE, &MELLOW] {
            for n in score.notes {
                let t = n.timbre;
                let mut v = Voice::start(n, score.amp);
                let mut pcm = Vec::new();
                while v.live {
                    let mut buf = [0i32; 160];
                    v.render(&mut buf);
                    pcm.extend_from_slice(&buf);
                    assert!(pcm.len() < 6 * RATE as usize, "{n:?} never ends");
                }
                // The attack: sample i is under the ramp.
                let full: i32 = t.partials.iter().map(|p| (i32::from(score.amp) * i32::from(p.amp)) >> 15).sum();
                assert_eq!(pcm[0], 0, "{n:?}");
                for (i, s) in pcm.iter().take(usize::from(t.attack)).enumerate() {
                    assert!(s.abs() <= full * (i as i32 + 1) / i32::from(t.attack) + 1, "{n:?} sample {i} is {s}");
                }
                // From its loudest 4 ms on, no 4 ms are less than 0.6 of
                // the 4 ms before until the note is inaudible (the fastest
                // release, 25 ms, gives 0.85): nothing is cut off.
                let peaks: Vec<i32> = pcm.chunks(64).map(|c| c.iter().map(|s| s.abs()).max().unwrap()).collect();
                let loudest = peaks.iter().enumerate().max_by_key(|(_, p)| **p).unwrap().0;
                for w in peaks[loudest..].windows(2) {
                    assert!(w[0] < 30 || w[1] * 10 >= w[0] * 6, "{n:?} falls from {} to {}", w[0], w[1]);
                }
                // The last thing it says is next to nothing.
                assert!(pcm[pcm.len() - 160..].iter().all(|s| s.abs() <= 4), "{n:?}");
            }
        }
    }
}
