// SPDX-License-Identifier: GPL-2.0-or-later
//! Ring playback: the built-in ringtones, synthesized from scores by
//! [`crate::ringsynth`] (the stock ringtones cannot be shipped; these are
//! original), and WAV files (PCM, mu-law, or A-law) for user ringtones.
//! The same player plays a sound file once for an app
//! ([`RingSource::Once`]).  Output is 16 kHz mono, 10 ms at a time.

use std::sync::Arc;

use crate::bands::WB;
use crate::g711::Law;
use crate::ringsynth::{self, Score, Synth};

pub const RING_RATE: u32 = ringsynth::RATE;

/// Built-in ringtones, all original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinRing {
    /// A short phrase on a marimba.
    Marimba,
    /// A telephone bell: two gongs under one clapper, two seconds on and
    /// four off.
    Classic,
    /// Three falling bell notes.
    Chime,
    /// Two short rounded beeps.
    Pulse,
    /// An electric piano's rising arpeggio.
    Rise,
    /// Two soft notes, for a quiet office.
    Mellow,
}

impl BuiltinRing {
    /// What a phone rings with when nothing else is chosen, and what a
    /// name that is no ringtone (any more) rings with.
    pub const DEFAULT: BuiltinRing = Self::Marimba;

    /// Every built-in ring, the default first.
    pub const ALL: [BuiltinRing; 6] = [Self::Marimba, Self::Classic, Self::Chime, Self::Pulse, Self::Rise, Self::Mellow];

    pub fn name(self) -> &'static str {
        match self {
            Self::Marimba => "Marimba",
            Self::Classic => "Classic",
            Self::Chime => "Chime",
            Self::Pulse => "Pulse",
            Self::Rise => "Rise",
            Self::Mellow => "Mellow",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name().eq_ignore_ascii_case(name))
    }

    /// The length of one cycle, the phrase and the pause after it.
    pub fn cycle_ms(self) -> u32 {
        u32::from(self.score().cycle_ms)
    }

    fn score(self) -> &'static Score {
        match self {
            Self::Marimba => &ringsynth::MARIMBA,
            Self::Classic => &ringsynth::CLASSIC,
            Self::Chime => &ringsynth::CHIME,
            Self::Pulse => &ringsynth::PULSE,
            Self::Rise => &ringsynth::RISE,
            Self::Mellow => &ringsynth::MELLOW,
        }
    }
}

/// What to ring with.
#[derive(Clone, Debug)]
pub enum RingSource {
    Builtin(BuiltinRing),
    /// 16 kHz samples, looped with a pause between passes.
    Pcm { samples: Arc<[i16]>, pause_ms: u32 },
    /// 16 kHz samples played once, from sample `start` to the end.
    /// `token` names the playback in [`RingPlayer::one_shot`].
    Once { samples: Arc<[i16]>, start: usize, token: u32 },
}

// The synthesizer's voices are kept in place: a ring starts on the audio
// thread, which does not allocate.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum State {
    Synth(Synth),
    Pcm { samples: Arc<[i16]>, pos: usize, pause: u32, pause_left: u32 },
    Once { samples: Arc<[i16]>, pos: usize, token: u32 },
}

/// Produces a ring, 160 samples (10 ms at 16 kHz) at a time.
#[derive(Clone, Debug)]
pub struct RingPlayer {
    state: State,
    /// Output gain, Q12.
    gain_q12: i32,
}

/// Where [`soft_limit`] starts to bend, and what it never exceeds.
const LIMIT_KNEE: i32 = 20_000;
const LIMIT_CEILING: i32 = 32_000;

/// Leaves samples up to [`LIMIT_KNEE`] alone and bends louder ones over
/// smoothly (the slope is continuous at the knee) so that none exceeds
/// [`LIMIT_CEILING`]: a built-in ring turned up past full scale rounds off
/// rather than clips.
fn soft_limit(v: i32) -> i16 {
    let mag = v.abs();
    if mag <= LIMIT_KNEE {
        return v as i16;
    }
    let room = LIMIT_CEILING - LIMIT_KNEE;
    let over = i64::from(mag - LIMIT_KNEE);
    let out = LIMIT_KNEE + (i64::from(room) * over / (i64::from(room) + over)) as i32;
    (if v < 0 { -out } else { out }) as i16
}

impl RingPlayer {
    pub fn new(source: RingSource) -> Self {
        let state = match source {
            RingSource::Builtin(b) => State::Synth(Synth::new(b.score())),
            RingSource::Pcm { samples, pause_ms } => State::Pcm { samples, pos: 0, pause: pause_ms * (RING_RATE / 1000), pause_left: 0 },
            RingSource::Once { samples, start, token } => {
                let pos = start.min(samples.len());
                State::Once { samples, pos, token }
            }
        };
        Self { state, gain_q12: 4096 }
    }

    pub fn set_gain_q12(&mut self, gain: i32) {
        self.gain_q12 = gain;
    }

    /// For a [`RingSource::Once`] source: its token, the sample position
    /// reached, and whether it has played to the end.
    pub fn one_shot(&self) -> Option<(u32, usize, bool)> {
        match &self.state {
            State::Once { samples, pos, token } => Some((*token, *pos, *pos >= samples.len())),
            _ => None,
        }
    }

    /// Fills `out` with the next 10 ms.
    pub fn fill(&mut self, out: &mut [i16; WB]) {
        match &mut self.state {
            State::Pcm { samples, pos, pause, pause_left } => {
                for o in out.iter_mut() {
                    *o = if *pause_left > 0 || samples.is_empty() {
                        *pause_left = pause_left.saturating_sub(1);
                        0
                    } else {
                        let s = samples[*pos];
                        *pos += 1;
                        if *pos >= samples.len() {
                            *pos = 0;
                            *pause_left = *pause;
                        }
                        s
                    };
                }
            }
            State::Once { samples, pos, .. } => {
                let n = (samples.len() - *pos).min(WB);
                out[..n].copy_from_slice(&samples[*pos..*pos + n]);
                out[n..].fill(0);
                *pos += n;
            }
            State::Synth(synth) => {
                // The gain goes on before the limiter, so that a loud ring
                // is rounded off rather than clipped.
                let mut mix = [0i32; WB];
                synth.mix_into(&mut mix);
                let gain = self.gain_q12.clamp(0, 8 << 12);
                for (o, m) in out.iter_mut().zip(mix) {
                    *o = soft_limit((m.clamp(-65_535, 65_535) * gain) >> 12);
                }
                return;
            }
        }
        crate::bands::scale(out, self.gain_q12);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WavError {
    NotWav,
    Truncated,
    /// Format tag, channels, or sample size this player does not handle.
    Unsupported(&'static str),
}

impl std::fmt::Display for WavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WavError::NotWav => f.write_str("not a RIFF/WAVE file"),
            WavError::Truncated => f.write_str("truncated WAVE file"),
            WavError::Unsupported(what) => write!(f, "unsupported WAVE file: {what}"),
        }
    }
}

impl std::error::Error for WavError {}

/// Decodes a WAV file (8-bit or 16-bit PCM, mu-law, or A-law; mono or the
/// first channel of more) to 16 kHz mono.  Not for the audio thread: it
/// allocates, and it resamples by linear interpolation.
pub fn decode_wav(data: &[u8]) -> Result<Vec<i16>, WavError> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(WavError::NotWav);
    }
    let u16_at = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |b: &[u8], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let mut fmt: Option<(u16, usize, u32, u16)> = None;
    let mut pcm: Option<&[u8]> = None;
    let mut o = 12;
    while o + 8 <= data.len() {
        let id = &data[o..o + 4];
        let len = u32_at(data, o + 4) as usize;
        let body = &data[o + 8..data.len().min(o + 8 + len)];
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err(WavError::Truncated);
                }
                fmt = Some((u16_at(body, 0), usize::from(u16_at(body, 2)), u32_at(body, 4), u16_at(body, 14)));
            }
            b"data" => pcm = Some(body),
            _ => {}
        }
        o += 8 + len + (len & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or(WavError::NotWav)?;
    let pcm = pcm.ok_or(WavError::Truncated)?;
    if channels == 0 || rate == 0 || rate > 192_000 {
        return Err(WavError::Unsupported("channel count or sample rate"));
    }
    let frame = channels * usize::from(bits / 8);
    let first: Vec<i16> = match (tag, bits) {
        (1, 16) => pcm.chunks_exact(frame).map(|f| i16::from_le_bytes([f[0], f[1]])).collect(),
        (1, 8) => pcm.chunks_exact(frame).map(|f| (i16::from(f[0]) - 128) << 8).collect(),
        (7, 8) => pcm.chunks_exact(frame).map(|f| Law::Mu.decode_sample(f[0])).collect(),
        (6, 8) => pcm.chunks_exact(frame).map(|f| Law::A.decode_sample(f[0])).collect(),
        _ => return Err(WavError::Unsupported("sample format")),
    };
    if rate == RING_RATE || first.is_empty() {
        return Ok(first);
    }
    // linear interpolation to 16 kHz, positions in Q16
    let out_len = (first.len() as u64 * u64::from(RING_RATE) / u64::from(rate)) as usize;
    let step = (u64::from(rate) << 16) / u64::from(RING_RATE);
    Ok((0..out_len)
        .map(|i| {
            let p = i as u64 * step;
            let (idx, frac) = ((p >> 16) as usize, (p & 0xffff) as i32);
            let a = i32::from(first[idx.min(first.len() - 1)]);
            let b = i32::from(first[(idx + 1).min(first.len() - 1)]);
            (a + (((b - a) * frac) >> 16)) as i16
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&tag.to_le_bytes());
        w.extend_from_slice(&channels.to_le_bytes());
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * u32::from(channels) * u32::from(bits) / 8).to_le_bytes());
        w.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        w.extend_from_slice(&bits.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(data);
        w
    }

    #[test]
    fn wav_formats() {
        let pcm: Vec<u8> = [100i16, -200, 300, -400].iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(decode_wav(&wav(1, 1, 16_000, 16, &pcm)).unwrap(), [100, -200, 300, -400]);
        // stereo: first channel
        assert_eq!(decode_wav(&wav(1, 2, 16_000, 16, &pcm)).unwrap(), [100, 300]);
        // 8 kHz is brought to 16 kHz
        let up = decode_wav(&wav(1, 1, 8_000, 16, &pcm)).unwrap();
        assert_eq!(up.len(), 8);
        assert_eq!((up[0], up[1], up[2]), (100, -50, -200));
        // mu-law
        let mu = [Law::Mu.encode_sample(1000), Law::Mu.encode_sample(-1000)];
        let d = decode_wav(&wav(7, 1, 16_000, 8, &mu)).unwrap();
        assert!((d[0] - 1000).abs() < 40 && (d[1] + 1000).abs() < 40);
        assert_eq!(decode_wav(b"nope"), Err(WavError::NotWav));
        assert_eq!(decode_wav(&wav(3, 1, 16_000, 32, &pcm)), Err(WavError::Unsupported("sample format")));
    }

    /// `cycles` cycles of a built-in ring at `gain_q12`.
    fn render(ring: BuiltinRing, cycles: u32, gain_q12: i32) -> Vec<i16> {
        let mut p = RingPlayer::new(RingSource::Builtin(ring));
        p.set_gain_q12(gain_q12);
        let mut pcm = Vec::new();
        for _ in 0..ring.cycle_ms() / 10 * cycles {
            let mut buf = [0i16; WB];
            p.fill(&mut buf);
            pcm.extend_from_slice(&buf);
        }
        pcm
    }

    fn rms(pcm: &[i16]) -> f64 {
        (pcm.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / pcm.len() as f64).sqrt()
    }

    #[test]
    fn builtin_ring_names() {
        assert_eq!(BuiltinRing::ALL[0], BuiltinRing::DEFAULT);
        for ring in BuiltinRing::ALL {
            assert_eq!(BuiltinRing::from_name(ring.name()), Some(ring));
            assert_eq!(BuiltinRing::from_name(&ring.name().to_uppercase()), Some(ring));
        }
        // The names saved settings may hold still name a ring.
        for name in ["Classic", "Chime", "Pulse", "Rise", "Mellow"] {
            assert!(BuiltinRing::from_name(name).is_some(), "{name}");
        }
        assert_eq!(BuiltinRing::from_name("Soft"), None);
        assert_eq!(BuiltinRing::from_name(""), None);
    }

    #[test]
    fn builtin_rings_are_a_phrase_and_a_pause() {
        for ring in BuiltinRing::ALL {
            let cycle = (ring.cycle_ms() * (RING_RATE / 1000)) as usize;
            assert!((3000..=6000).contains(&ring.cycle_ms()) && ring.cycle_ms() % 10 == 0, "{ring:?}");
            let pcm = render(ring, 3, 4096);
            assert_eq!(pcm.len(), 3 * cycle);
            let one = &pcm[..cycle];

            // No clipping, and nowhere near the limiter at unity gain.
            let peak = one.iter().map(|s| i32::from(*s).abs()).max().unwrap();
            assert!((6000..=16_000).contains(&peak), "{ring:?} peak {peak}");

            // It starts from silence, and ends in silence: at least two
            // seconds more than 50 dB below its peak, the last half second
            // of them nothing at all, before it comes round.
            assert_eq!(one[0], 0, "{ring:?}");
            let last = one.iter().rposition(|s| i32::from(s.abs()) > peak / 300).unwrap();
            assert!(cycle - last >= 32_000, "{ring:?} sounds until {last} of {cycle}");
            assert!(one[cycle - 8000..].iter().all(|&s| s == 0), "{ring:?}");
            let first_loud = one.iter().position(|s| s.abs() > 500).unwrap();
            let last_loud = one.iter().rposition(|s| s.abs() > 500).unwrap();
            assert!(first_loud < 800, "{ring:?} is late: {first_loud}");
            assert!(last_loud > 4000, "{ring:?} is too short: {last_loud}");

            // It loops seamlessly: every cycle is the first one again.
            assert_eq!(one, &pcm[cycle..2 * cycle], "{ring:?} second cycle");
            assert_eq!(one, &pcm[2 * cycle..], "{ring:?} third cycle");

            // No clicks (`ringsynth` tests each note's start and end): the
            // wave never jumps by as much as its own peak.
            let step = one.windows(2).map(|w| (i32::from(w[1]) - i32::from(w[0])).abs()).max().unwrap();
            assert!(step < peak, "{ring:?} steps by {step}, peak {peak}");

            // About as loud as the rings these replace.  The old default,
            // a 9000-peak warble of 440 Hz and 480 Hz, was 6340 RMS, but
            // at the bottom of what the loudspeaker gives; through a 450 Hz
            // high-pass filter and A-weighting it measured 3000, and the
            // old rings near 1 kHz 3500 to 5400, where these measure 3800
            // to 4500 (and Mellow, the soft one, 2900).  Unweighted, the
            // loudest 400 ms of each:
            let loudest = one.chunks(6400).map(rms).fold(0.0, f64::max);
            assert!((3200.0..=4600.0).contains(&loudest), "{ring:?} loudest 400 ms {loudest:.0}");
        }
    }

    #[test]
    fn builtin_rings_fit_the_voices() {
        // The player is made and kept on the audio thread, in place.
        assert!(std::mem::size_of::<RingPlayer>() <= 1024, "{}", std::mem::size_of::<RingPlayer>());
        for ring in BuiltinRing::ALL {
            let mut synth = Synth::new(ring.score());
            for _ in 0..ring.cycle_ms() / 10 * 2 {
                synth.mix_into(&mut [0i32; WB]);
            }
            let (dropped, most) = synth.load();
            assert_eq!(dropped, 0, "{ring:?}");
            assert!((1..=ringsynth::VOICES).contains(&most), "{ring:?} {most}");
        }
    }

    #[test]
    fn a_loud_ring_is_limited_not_clipped() {
        assert_eq!(soft_limit(0), 0);
        assert_eq!(soft_limit(LIMIT_KNEE), LIMIT_KNEE as i16);
        assert_eq!(soft_limit(-12_345), -12_345);
        let mut last = 0;
        for v in (0..400_000).step_by(7) {
            let out = i32::from(soft_limit(v));
            assert!(out >= last && out <= LIMIT_CEILING, "{v} -> {out}");
            assert_eq!(i32::from(soft_limit(-v)), -out);
            last = out;
        }
        // The top of the volume scale is 15 dB above unity.
        for ring in BuiltinRing::ALL {
            let pcm = render(ring, 1, 23_033);
            let peak = pcm.iter().map(|s| i32::from(*s).abs()).max().unwrap();
            assert!(peak > LIMIT_KNEE && peak <= LIMIT_CEILING, "{ring:?} peak {peak}");
            // Unity is the same samples scaled.
            let quiet = render(ring, 1, 2048);
            let unity = render(ring, 1, 4096);
            assert!(quiet.iter().zip(&unity).all(|(q, u)| (i32::from(*q) - i32::from(*u) / 2).abs() <= 1), "{ring:?}");
        }
        assert!(render(BuiltinRing::DEFAULT, 1, 0).iter().all(|&s| s == 0));
    }

    #[test]
    fn one_shot_plays_from_its_start_to_the_end() {
        let samples: Arc<[i16]> = (0..400).map(|i| i as i16 + 1).collect();
        let mut p = RingPlayer::new(RingSource::Once { samples: samples.clone(), start: 100, token: 7 });
        assert_eq!(p.one_shot(), Some((7, 100, false)));
        let mut buf = [0i16; WB];
        p.fill(&mut buf);
        assert_eq!((buf[0], buf[159]), (101, 260));
        assert_eq!(p.one_shot(), Some((7, 260, false)));
        p.fill(&mut buf);
        assert_eq!((buf[0], buf[139]), (261, 400));
        assert!(buf[140..].iter().all(|&s| s == 0));
        assert_eq!(p.one_shot(), Some((7, 400, true)));
        p.fill(&mut buf);
        assert!(buf.iter().all(|&s| s == 0));
        // A start beyond the end is the end.
        let p = RingPlayer::new(RingSource::Once { samples, start: 9000, token: 1 });
        assert_eq!(p.one_shot(), Some((1, 400, true)));
        assert_eq!(RingPlayer::new(RingSource::Builtin(BuiltinRing::Classic)).one_shot(), None);
    }

    #[test]
    fn pcm_ring_loops() {
        let samples: Arc<[i16]> = (0..240).map(|i| i as i16 + 1).collect();
        let mut p = RingPlayer::new(RingSource::Pcm { samples, pause_ms: 10 });
        let mut buf = [0i16; WB];
        p.fill(&mut buf);
        assert_eq!(buf[159], 160);
        p.fill(&mut buf);
        assert_eq!(buf[79], 240);
        assert!(buf[80..].iter().all(|&s| s == 0));
        p.fill(&mut buf);
        assert!(buf[..80].iter().all(|&s| s == 0));
        assert_eq!(buf[80], 1);
    }
}
