// SPDX-License-Identifier: GPL-2.0-or-later
//! Ring playback: a few built-in ringtones synthesized from note lists (the
//! stock ringtones cannot be shipped), and WAV files (PCM, mu-law, or A-law)
//! for user ringtones.  The same player plays a sound file once for an app
//! ([`RingSource::Once`]).  Output is 16 kHz mono, 10 ms at a time.

use std::sync::Arc;

use crate::bands::{WB, sat16};
use crate::g711::Law;
use crate::tone::Osc;

pub const RING_RATE: u32 = 16_000;

/// One note of a built-in ringtone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    /// Two summed frequencies in Hz; 0 = unused.  Both 0 is a rest.
    pub f1: u16,
    pub f2: u16,
    pub ms: u16,
    /// Decay: the envelope loses 1/2^decay of itself every millisecond;
    /// 0 = no decay.
    pub decay: u8,
}

const fn n(f1: u16, f2: u16, ms: u16, decay: u8) -> Note {
    Note { f1, f2, ms, decay }
}

/// Built-in ringtones, all original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinRing {
    /// A two-tone warble, two seconds on and four off.
    Classic,
    /// Three falling bell notes.
    Chime,
    /// Two short beeps.
    Pulse,
    /// A rising arpeggio.
    Rise,
    /// A soft low double note.
    Mellow,
}

impl BuiltinRing {
    pub const ALL: [BuiltinRing; 5] = [Self::Classic, Self::Chime, Self::Pulse, Self::Rise, Self::Mellow];

    pub fn name(self) -> &'static str {
        match self {
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

    fn notes(self) -> &'static [Note] {
        const WARBLE: [Note; 2] = [n(440, 0, 25, 0), n(480, 0, 25, 0)];
        const CLASSIC: [Note; 81] = {
            let mut a = [n(0, 0, 0, 0); 81];
            let mut i = 0;
            while i < 80 {
                a[i] = WARBLE[i % 2];
                i += 1;
            }
            a[80] = n(0, 0, 4000, 0);
            a
        };
        const CHIME: [Note; 4] = [n(1319, 2637, 350, 7), n(1047, 2093, 350, 7), n(784, 1568, 700, 8), n(0, 0, 2600, 0)];
        const PULSE: [Note; 4] = [n(1000, 0, 150, 0), n(0, 0, 100, 0), n(1000, 0, 150, 0), n(0, 0, 2600, 0)];
        const RISE: [Note; 6] =
            [n(523, 0, 110, 6), n(659, 0, 110, 6), n(784, 0, 110, 6), n(1047, 0, 110, 6), n(1319, 0, 300, 7), n(0, 0, 2400, 0)];
        const MELLOW: [Note; 4] = [n(392, 784, 400, 8), n(0, 0, 150, 0), n(330, 659, 600, 8), n(0, 0, 2850, 0)];
        match self {
            Self::Classic => &CLASSIC,
            Self::Chime => &CHIME,
            Self::Pulse => &PULSE,
            Self::Rise => &RISE,
            Self::Mellow => &MELLOW,
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

#[derive(Clone, Debug)]
enum State {
    Notes { notes: &'static [Note], idx: usize, left: u32, osc: [Osc; 2], env: i32, ms_tick: u32 },
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

const NOTE_AMP: i32 = 9000;

impl RingPlayer {
    pub fn new(source: RingSource) -> Self {
        let state = match source {
            RingSource::Builtin(b) => {
                let notes = b.notes();
                let mut s = State::Notes { notes, idx: 0, left: 0, osc: [Osc::new(0, RING_RATE, 0), Osc::new(0, RING_RATE, 0)], env: 0, ms_tick: 0 };
                Self::load_note(&mut s);
                s
            }
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

    fn load_note(s: &mut State) {
        if let State::Notes { notes, idx, left, osc, env, ms_tick } = s {
            let note = notes[*idx];
            *left = u32::from(note.ms) * (RING_RATE / 1000);
            osc[0] = Osc::new(u32::from(note.f1), RING_RATE, if note.f1 == 0 { 0 } else { NOTE_AMP });
            osc[1] = Osc::new(u32::from(note.f2), RING_RATE, if note.f2 == 0 { 0 } else { NOTE_AMP / 2 });
            *env = 1 << 15;
            *ms_tick = 0;
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
            State::Notes { .. } => {
                for o in out.iter_mut() {
                    let State::Notes { notes, idx, left, osc, env, ms_tick } = &mut self.state else { unreachable!() };
                    if *left == 0 {
                        *idx = (*idx + 1) % notes.len();
                        Self::load_note(&mut self.state);
                        *o = 0;
                        continue;
                    }
                    let note = notes[*idx];
                    let s = i32::from(osc[0].next()) + i32::from(osc[1].next());
                    *o = sat16((s * *env) >> 15);
                    *left -= 1;
                    *ms_tick += 1;
                    if *ms_tick == RING_RATE / 1000 {
                        *ms_tick = 0;
                        if note.decay != 0 {
                            *env -= *env >> note.decay;
                        }
                    }
                }
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

    #[test]
    fn builtin_rings_sound_and_pause() {
        for ring in BuiltinRing::ALL {
            assert_eq!(BuiltinRing::from_name(ring.name()), Some(ring));
            let mut p = RingPlayer::new(RingSource::Builtin(ring));
            let mut loud = 0;
            let mut quiet = 0;
            let mut peak = 0i32;
            for _ in 0..700 {
                let mut buf = [0i16; WB];
                p.fill(&mut buf);
                let m = buf.iter().map(|s| i32::from(*s).abs()).max().unwrap();
                peak = peak.max(m);
                if m > 500 { loud += 1 } else { quiet += 1 }
            }
            assert!(loud > 20, "{ring:?} loud {loud}");
            assert!(quiet > 150, "{ring:?} quiet {quiet}");
            assert!(peak < 20_000, "{ring:?} peak {peak}");
        }
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
