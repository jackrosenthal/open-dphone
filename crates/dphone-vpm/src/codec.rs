// SPDX-License-Identifier: GPL-2.0-or-later
//! Codecs that run as DSP jobs.
//!
//! Only G.722 (64 kbit/s) and G.726-32 have been observed on hardware: their
//! codec numbers, init bodies, and frame layouts are known.  Every other
//! codec the DSP image contains needs an init body that was never captured,
//! so asking for it fails cleanly and call setup can leave it out of an
//! offer.  G.711 and linear PCM are not DSP jobs at all (the stock firmware
//! codes them on the ARM as well).

use crate::{Error, Family, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    G722,
    G726_32,
    G726_16,
    G726_24,
    G726_40,
    G729,
    G729A,
    G7231,
    G728,
    Ilbc20,
    Ilbc30,
    AmrNb,
    AmrWb,
    G7221,
    G7111,
    Opus,
    Isac,
}

/// What the host must know to run a codec's jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodecParams {
    /// Codec number in this family's table.
    pub number: u16,
    /// PCM sample rate of the job bodies.
    pub pcm_rate: u32,
    /// Samples per job.
    pub frame_samples: usize,
    /// Encoded bytes per job.
    pub frame_bytes: usize,
    /// Jobs per 10 ms tick.
    pub frames_per_tick: usize,
    pub enc_cfg: [u16; 3],
    pub dec_cfg: [u16; 2],
}

impl CodecParams {
    pub fn pcm_bytes(&self) -> usize {
        2 * self.frame_samples
    }
}

impl Codec {
    /// Every codec the image may contain; the capability list for call
    /// setup is this filtered by [`Codec::params`] succeeding.
    pub const ALL: [Codec; 17] = [
        Codec::G722,
        Codec::G726_32,
        Codec::G726_16,
        Codec::G726_24,
        Codec::G726_40,
        Codec::G729,
        Codec::G729A,
        Codec::G7231,
        Codec::G728,
        Codec::Ilbc20,
        Codec::Ilbc30,
        Codec::AmrNb,
        Codec::AmrWb,
        Codec::G7221,
        Codec::G7111,
        Codec::Opus,
        Codec::Isac,
    ];

    /// Job parameters, or `Unsupported` for a codec whose init body was not
    /// captured.
    pub fn params(self, family: Family) -> Result<CodecParams> {
        match self {
            // 10 ms frames: 160 samples of 16 kHz PCM against 80 bytes.
            Codec::G722 => Ok(CodecParams {
                number: match family {
                    Family::New => 46,
                    Family::Old => 40,
                },
                pcm_rate: 16_000,
                frame_samples: 160,
                frame_bytes: 80,
                frames_per_tick: 1,
                enc_cfg: [0x0000, 0x0000, 0x0001],
                dec_cfg: [0x0000, 0x0001],
            }),
            // 5 ms frames: 40 samples of 8 kHz PCM against 20 bytes.  The
            // decoder init body was not captured; the G.722 one is assumed.
            Codec::G726_32 => Ok(CodecParams {
                number: 4,
                pcm_rate: 8_000,
                frame_samples: 40,
                frame_bytes: 20,
                frames_per_tick: 2,
                enc_cfg: [0x0002, 0x0000, 0x0001],
                dec_cfg: [0x0000, 0x0001],
            }),
            _ => Err(Error::Unsupported("init body not captured")),
        }
    }

    pub fn is_supported(self, family: Family) -> bool {
        self.params(family).is_ok()
    }

    /// Codecs usable in a call on this family.
    pub fn supported(family: Family) -> impl Iterator<Item = Codec> {
        Self::ALL.into_iter().filter(move |c| c.is_supported(family))
    }
}

/// The DSP packs an encoded stream big-endian into its 16-bit words, so in
/// a job body each pair of bytes is swapped against RTP order, in both
/// directions (measured for G.722 on both families; assumed for G.726).
/// This converts either way, in place.  A trailing odd byte is left alone.
pub fn swap_pairs(frame: &mut [u8]) {
    for pair in frame.as_chunks_mut::<2>().0 {
        pair.swap(0, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_captured_codecs_are_supported() {
        for family in [Family::Old, Family::New] {
            let ok: Vec<_> = Codec::supported(family).collect();
            assert_eq!(ok, [Codec::G722, Codec::G726_32]);
            assert_eq!(Codec::Opus.params(family), Err(Error::Unsupported("init body not captured")));
        }
        assert_eq!(Codec::G722.params(Family::New).unwrap().number, 46);
        assert_eq!(Codec::G722.params(Family::Old).unwrap().number, 40);
    }

    #[test]
    fn pair_swap() {
        let mut f = [1, 2, 3, 4, 5];
        swap_pairs(&mut f);
        assert_eq!(f, [2, 1, 4, 3, 5]);
    }
}
