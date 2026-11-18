// SPDX-License-Identifier: GPL-2.0-or-later
//! Job wrappers, one per job the stock firmware sends.
//!
//! Channel and instance numbers are always the caller's; [`crate::ChannelMap`]
//! has the vendor's map, which is the safe choice.

use crate::abi::{self, Info};
use crate::transport::{Reply, Request, Transport};
use crate::{Error, Family, Result};

/// PVE (vocoder) operations.
pub mod pve {
    pub const ENC_INIT: u16 = 0;
    pub const ENCODE: u16 = 2;
    pub const DEC_INIT: u16 = 3;
    pub const DECODE: u16 = 5;
    pub const QUERY_CODEC: u16 = 6;
    pub const QUERY_RATE_CODE: u16 = 7;
    /// Fixed decode request buffers (measured for G.722 and G.726-32).
    pub const DEC_BUF_NEW: usize = 512;
    pub const DEC_BUF_OLD: usize = 160;
    /// Bytes per entry of the rate-code list.
    pub const RATE_CODE_ENTRY: usize = 4;
    /// Bytes of a codec-properties reply.
    pub const CODEC_PROPS: usize = 536;
}

/// HEC (handset and headset canceller) operations.
pub mod hec {
    pub const INIT: u16 = 0;
    /// Never sent by the vendor in any trace.
    pub const DEINIT: u16 = 1;
    pub const INGRESS: u16 = 2;
    pub const EGRESS: u16 = 3;
    pub const ENABLE: u16 = 4;
    pub const SET_REGS: u16 = 5;
    pub const SET_REG: u16 = 6;
    pub const GET_REGS: u16 = 7;
    pub const GET_STATE: u16 = 8;
    pub const RESET: u16 = 9;
    pub const GET_STATS: u16 = 14;
    pub const NUM_REGS: usize = 17;
    /// Register 0 holds option bits.  The vendor sets these two while the
    /// path plays a local tone ("tone mode") and clears them when the
    /// tone stops; read back as 0x8002 on a stock D60 with the dial tone
    /// on the headset, and 0 otherwise.
    pub const REG_OPTIONS: u16 = 0;
    pub const OPTIONS_TONE: u16 = 0x8002;
    /// Per-frame body: 10 ms as three band buffers (new) or two (old).
    pub const FRAME_NEW: usize = 1120;
    pub const FRAME_OLD: usize = 480;
    pub const STATE_BYTES: usize = 48;
    /// "Register 4" holds the receive gain in dB * 256 (inferred).
    pub const REG_RX_GAIN: u16 = 4;
}

/// SBAEC (speakerphone canceller) operations.
pub mod sbaec {
    pub const INIT: u16 = 0;
    pub const ENABLE: u16 = 1;
    pub const RESET: u16 = 2;
    pub const RESET_COLD: u16 = 3;
    pub const SET_REG: u16 = 4;
    pub const SET_REGS: u16 = 5;
    pub const GET_REGS: u16 = 6;
    pub const GET_STATE: u16 = 7;
    pub const EGRESS: u16 = 8;
    pub const INGRESS: u16 = 9;
    /// Never sent by the vendor in any trace.
    pub const DEINIT: u16 = 13;
    pub const GET_REG: u16 = 16;
    pub const GET_STATS: u16 = 17;
    pub const NUM_REGS_NEW: usize = 932;
    pub const NUM_REGS_OLD: usize = 931;
    /// 10 ms as two band buffers, both families.
    pub const FRAME: usize = 480;
    pub const FRAME_SAMPLES: u16 = 80;
    pub const STATE_BYTES: usize = 40;
    /// Register 0 holds option bits.  The vendor sets bit 1, and bit 15
    /// of the register [`tone_reg`] names, while the speakerphone plays a
    /// local tone ("tone mode"), and clears both when the tone stops
    /// (read back from a stock D60: 2 and 0x8020 with the dial tone on,
    /// 0 and 0x0020 after).
    pub const REG_OPTIONS: u16 = 0;
    pub const OPTIONS_TONE: u16 = 0x0002;
    pub const TONE_REG_BIT: u16 = 0x8000;
    /// Receive gain in dB * 256.
    pub const REG_RX_GAIN: u16 = 2;
    /// Bit 0 of a direction's flag register: the frame job takes and
    /// returns both band buffers.  Without it the DSP returns only the
    /// lower band (twice argument 1 in bytes).  The vendor's canceller
    /// service writes 1 to both registers before its first frames;
    /// measured on a D60, on stock and on this driver.
    pub const FLAG_BOTH_BANDS: u16 = 1;

    use crate::Family;

    /// The second register of tone mode (the old family's register file
    /// is one word shorter below it).
    pub const fn tone_reg(family: Family) -> u16 {
        match family {
            Family::New => 330,
            Family::Old => 329,
        }
    }

    /// The flag register of the microphone-side frame job.
    pub const fn ingress_flags_reg(family: Family) -> u16 {
        match family {
            Family::New => 0x300,
            Family::Old => 0x2ff,
        }
    }

    /// The flag register of the loudspeaker-side frame job.
    pub const fn egress_flags_reg(family: Family) -> u16 {
        match family {
            Family::New => 0x301,
            Family::Old => 0x300,
        }
    }
}

/// BAND (filter bank, new family only) operations.
pub mod band {
    pub const SPLIT_INIT: u16 = 0;
    pub const SPLIT_SWB: u16 = 1;
    pub const SPLIT_WB: u16 = 2;
    pub const COMBINE_INIT: u16 = 3;
    pub const COMBINE_SWB: u16 = 4;
    pub const COMBINE_WB: u16 = 5;
}

/// The DSP's words are little-endian; so is every CPU these phones have.
pub fn words_to_bytes(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// A raw job, for diagnostics and replay.
#[derive(Clone, Copy, Debug, Default)]
pub struct Call<'a> {
    pub channel: u16,
    pub client: u16,
    pub instance: u16,
    pub opcode: u16,
    pub codec: u16,
    pub arg1: u16,
    pub arg2: u16,
    pub body: &'a [u8],
    pub timeout_ms: u32,
}

/// A handle on the DSP job interface.
pub struct Vpm<T: Transport> {
    transport: T,
    info: Info,
    family: Family,
}

impl<T: Transport> Vpm<T> {
    /// Takes a transport, reads the driver's info, and checks the ABI.
    pub fn new(mut transport: T) -> Result<Self> {
        let info = transport.info()?;
        if info.abi_version != abi::ABI_VERSION {
            return Err(Error::AbiVersion(info.abi_version));
        }
        let family = Family::from_abi(info.family).ok_or(Error::Invalid)?;
        Ok(Self { transport, info, family })
    }

    pub fn family(&self) -> Family {
        self.family
    }

    pub fn is_new_family(&self) -> bool {
        self.family == Family::New
    }

    /// Info as of the last refresh.
    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn refresh_info(&mut self) -> Result<&Info> {
        self.info = self.transport.info()?;
        Ok(&self.info)
    }

    pub fn is_online(&self) -> bool {
        self.info.state == abi::STATE_ONLINE
    }

    pub fn transport(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    /// One job, as given.
    pub fn job(&mut self, c: &Call<'_>, reply: &mut [u8]) -> Result<Reply> {
        self.transport.job(
            &Request {
                channel: c.channel,
                client: c.client,
                instance: c.instance,
                opcode: c.opcode,
                codec: c.codec,
                arg1: c.arg1,
                arg2: c.arg2,
                body: c.body,
                timeout_ms: c.timeout_ms,
            },
            reply,
        )
    }

    /// A job whose result must fill at least `want` bytes of `reply`.
    fn exact(&mut self, c: &Call<'_>, reply: &mut [u8], want: usize) -> Result<Reply> {
        let r = self.job(c, reply)?;
        if r.len < want {
            return Err(Error::ShortReply { want, got: r.len, client: c.client, channel: c.channel, opcode: c.opcode });
        }
        Ok(r)
    }

    #[allow(clippy::too_many_arguments)]
    fn ctl(&mut self, client: u16, channel: u16, instance: u16, opcode: u16, arg1: u16, arg2: u16, words: &[u16]) -> Result<Reply> {
        let body = words_to_bytes(words);
        self.job(&Call { channel, client, instance, opcode, arg1, arg2, body: &body, ..Call::default() }, &mut [])
    }

    /// A frame job done in place: `frame` goes in and is replaced by the
    /// result.  Now and then the DSP returns 6 more bytes than it was given
    /// (seen on canceller ingress jobs); they are dropped.
    fn frame(&mut self, client: u16, channel: u16, instance: u16, opcode: u16, arg1: u16, frame: &mut [u8]) -> Result<()> {
        let mut out = [0u8; hec::FRAME_NEW + 8];
        let want = frame.len();
        if want > hec::FRAME_NEW {
            return Err(Error::TooLong);
        }
        self.exact(&Call { channel, client, instance, opcode, arg1, body: frame, ..Call::default() }, &mut out[..want + 8], want)?;
        frame.copy_from_slice(&out[..want]);
        Ok(())
    }

    fn fetch(&mut self, client: u16, channel: u16, instance: u16, opcode: u16, out: &mut [u8]) -> Result<usize> {
        let r = self.job(&Call { channel, client, instance, opcode, ..Call::default() }, out)?;
        Ok(r.stored(out.len()))
    }

    // ---- client 1: PVE vocoders ------------------------------------------

    /// Op 7 with codec 0, sent by the vendor on a stream's decode channel:
    /// the firmware's own codec list.  Returns (entries, bytes stored); 76
    /// entries on the new family and 60 on the old, 4 bytes per entry.
    pub fn pve_query_rate_codes(&mut self, channel: u16, instance: u16, list: &mut [u8]) -> Result<(u16, usize)> {
        let r = self.job(
            &Call { channel, client: abi::CLIENT_PVE, instance, opcode: pve::QUERY_RATE_CODE, ..Call::default() },
            list,
        )?;
        Ok((r.value1, r.stored(list.len())))
    }

    /// Op 6: codec properties (536 bytes, mostly not decoded).
    pub fn pve_query_codec(&mut self, channel: u16, instance: u16, codec: u16, props: &mut [u8]) -> Result<usize> {
        let r = self.job(
            &Call { channel, client: abi::CLIENT_PVE, instance, opcode: pve::QUERY_CODEC, codec, ..Call::default() },
            props,
        )?;
        Ok(r.stored(props.len()))
    }

    /// Op 0 on the encode channel; `cfg` is three words.
    pub fn pve_enc_init(&mut self, channel: u16, instance: u16, codec: u16, cfg: &[u16; 3]) -> Result<()> {
        let body = words_to_bytes(cfg);
        self.job(&Call { channel, client: abi::CLIENT_PVE, instance, opcode: pve::ENC_INIT, codec, body: &body, ..Call::default() }, &mut [])?;
        Ok(())
    }

    /// Op 3 on the decode channel; `cfg` is two words.
    pub fn pve_dec_init(&mut self, channel: u16, instance: u16, codec: u16, cfg: &[u16; 2]) -> Result<()> {
        let body = words_to_bytes(cfg);
        self.job(&Call { channel, client: abi::CLIENT_PVE, instance, opcode: pve::DEC_INIT, codec, body: &body, ..Call::default() }, &mut [])?;
        Ok(())
    }

    /// Op 2: one frame of PCM in, one encoded frame out, in the DSP's word
    /// order.  `flags` is argument 1 (0 for G.722 and G.726).  Returns the
    /// encoded length and the status (1 in every frame seen).
    pub fn pve_encode(&mut self, channel: u16, instance: u16, codec: u16, flags: u16, pcm: &[u8], out: &mut [u8]) -> Result<(usize, u16)> {
        let r = self.job(
            &Call { channel, client: abi::CLIENT_PVE, instance, opcode: pve::ENCODE, codec, arg1: flags, body: pcm, ..Call::default() },
            out,
        )?;
        Ok((r.bytes(out.len()), r.value1))
    }

    /// Op 5: one encoded frame in (the DSP's word order), PCM out.
    /// `frame_type` is argument 1 (1 for a normal frame; 6 was seen once,
    /// meaning unknown).  The request body is the family's fixed buffer with
    /// the frame at its start; only the new family passes the valid length.
    pub fn pve_decode(&mut self, channel: u16, instance: u16, codec: u16, frame_type: u16, frame: &[u8], pcm: &mut [u8]) -> Result<usize> {
        let mut buf = [0u8; pve::DEC_BUF_NEW];
        let new = self.is_new_family();
        let body_len = if new { pve::DEC_BUF_NEW } else { pve::DEC_BUF_OLD };
        if frame.len() > body_len {
            return Err(Error::TooLong);
        }
        buf[..frame.len()].copy_from_slice(frame);
        let r = self.job(
            &Call {
                channel,
                client: abi::CLIENT_PVE,
                instance,
                opcode: pve::DECODE,
                codec,
                arg1: frame_type,
                arg2: if new { frame.len() as u16 } else { 0 },
                body: &buf[..body_len],
                ..Call::default()
            },
            pcm,
        )?;
        Ok(r.bytes(pcm.len()))
    }

    // ---- client 4: HEC ----------------------------------------------------

    pub fn hec_frame_bytes(&self) -> usize {
        if self.is_new_family() { hec::FRAME_NEW } else { hec::FRAME_OLD }
    }

    /// The Init body is a register file (17 words); arguments 0, 0.
    pub fn hec_init(&mut self, channel: u16, instance: u16, regs: &[u16]) -> Result<()> {
        self.ctl(abi::CLIENT_HEC, channel, instance, hec::INIT, 0, 0, regs).map(drop)
    }

    pub fn hec_enable(&mut self, channel: u16, instance: u16, on: bool) -> Result<()> {
        self.ctl(abi::CLIENT_HEC, channel, instance, hec::ENABLE, on as u16, 0, &[]).map(drop)
    }

    /// Whole register file; arguments 0, 0 as captured.
    pub fn hec_set_regs(&mut self, channel: u16, instance: u16, regs: &[u16]) -> Result<()> {
        self.ctl(abi::CLIENT_HEC, channel, instance, hec::SET_REGS, 0, 0, regs).map(drop)
    }

    pub fn hec_set_reg(&mut self, channel: u16, instance: u16, reg: u16, value: u16) -> Result<()> {
        self.ctl(abi::CLIENT_HEC, channel, instance, hec::SET_REG, reg, value, &[]).map(drop)
    }

    /// Microphone side, in place; `frame` is [`Self::hec_frame_bytes`] long.
    pub fn hec_ingress(&mut self, channel: u16, instance: u16, frame: &mut [u8]) -> Result<()> {
        self.frame(abi::CLIENT_HEC, channel, instance, hec::INGRESS, 0, frame)
    }

    /// Receiver side, in place.
    pub fn hec_egress(&mut self, channel: u16, instance: u16, frame: &mut [u8]) -> Result<()> {
        self.frame(abi::CLIENT_HEC, channel, instance, hec::EGRESS, 0, frame)
    }

    /// 48-byte state block, not decoded; the vendor polls it every frame.
    pub fn hec_get_state(&mut self, channel: u16, instance: u16, state: &mut [u8]) -> Result<usize> {
        self.fetch(abi::CLIENT_HEC, channel, instance, hec::GET_STATE, state)
    }

    /// Internal statistics (6 bytes seen), sent by the vendor once a second.
    pub fn hec_get_stats(&mut self, channel: u16, instance: u16, stats: &mut [u8]) -> Result<usize> {
        self.fetch(abi::CLIENT_HEC, channel, instance, hec::GET_STATS, stats)
    }

    // ---- client 3: SBAEC --------------------------------------------------

    pub fn sbaec_num_regs(&self) -> usize {
        if self.is_new_family() { sbaec::NUM_REGS_NEW } else { sbaec::NUM_REGS_OLD }
    }

    /// The Init body is a register file; `arg1` is 2 in the vendor's code.
    pub fn sbaec_init(&mut self, channel: u16, instance: u16, arg1: u16, regs: &[u16]) -> Result<()> {
        self.ctl(abi::CLIENT_SBAEC, channel, instance, sbaec::INIT, arg1, 0, regs).map(drop)
    }

    pub fn sbaec_enable(&mut self, channel: u16, instance: u16, on: bool) -> Result<()> {
        self.ctl(abi::CLIENT_SBAEC, channel, instance, sbaec::ENABLE, on as u16, 0, &[]).map(drop)
    }

    pub fn sbaec_set_reg(&mut self, channel: u16, instance: u16, reg: u16, value: u16) -> Result<()> {
        self.ctl(abi::CLIENT_SBAEC, channel, instance, sbaec::SET_REG, reg, value, &[]).map(drop)
    }

    /// Registers `first..first + regs.len()`; arguments are first and last.
    pub fn sbaec_set_regs(&mut self, channel: u16, instance: u16, first: u16, regs: &[u16]) -> Result<()> {
        if regs.is_empty() {
            return Err(Error::Invalid);
        }
        let last = first + regs.len() as u16 - 1;
        self.ctl(abi::CLIENT_SBAEC, channel, instance, sbaec::SET_REGS, first, last, regs).map(drop)
    }

    /// Microphone side, in place; `frame` is [`sbaec::FRAME`] bytes.
    ///
    /// Argument 1 is the frame's sample count at 8 kHz (80), as the
    /// vendor's wrapper passes it.  The whole frame comes back only while
    /// bit 0 of the direction's flag register is set (see
    /// [`sbaec::FLAG_BOTH_BANDS`]); the start-up sequence sets it.
    pub fn sbaec_ingress(&mut self, channel: u16, instance: u16, frame: &mut [u8]) -> Result<()> {
        self.frame(abi::CLIENT_SBAEC, channel, instance, sbaec::INGRESS, sbaec::FRAME_SAMPLES, frame)
    }

    /// Loudspeaker side, in place; as [`Self::sbaec_ingress`].
    pub fn sbaec_egress(&mut self, channel: u16, instance: u16, frame: &mut [u8]) -> Result<()> {
        self.frame(abi::CLIENT_SBAEC, channel, instance, sbaec::EGRESS, sbaec::FRAME_SAMPLES, frame)
    }

    /// 40-byte state block, not decoded.
    pub fn sbaec_get_state(&mut self, channel: u16, instance: u16, state: &mut [u8]) -> Result<usize> {
        self.fetch(abi::CLIENT_SBAEC, channel, instance, sbaec::GET_STATE, state)
    }

    /// Two signed result values, meaning not established (levels, by the
    /// look); the vendor asks once a second.
    pub fn sbaec_get_stats(&mut self, channel: u16, instance: u16) -> Result<(i16, i16)> {
        let r = self.job(&Call { channel, client: abi::CLIENT_SBAEC, instance, opcode: sbaec::GET_STATS, ..Call::default() }, &mut [])?;
        Ok((r.value1 as i16, r.value2 as i16))
    }

    // ---- client 8: BAND (new family only) ---------------------------------

    fn band_ok(&self, n: usize) -> Result<()> {
        if !self.is_new_family() {
            return Err(Error::NotOnThisFamily);
        }
        if n == 0 || n > 480 {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    /// Op 0 on a split channel; carries nothing.
    pub fn band_split_init(&mut self, channel: u16, instance: u16) -> Result<()> {
        self.band_ok(1)?;
        self.ctl(abi::CLIENT_BAND, channel, instance, band::SPLIT_INIT, 0, 0, &[]).map(drop)
    }

    /// Op 3 on a combine channel; carries nothing.
    pub fn band_combine_init(&mut self, channel: u16, instance: u16) -> Result<()> {
        self.band_ok(1)?;
        self.ctl(abi::CLIENT_BAND, channel, instance, band::COMBINE_INIT, 0, 0, &[]).map(drop)
    }

    #[allow(clippy::too_many_arguments)]
    fn band_job(&mut self, channel: u16, instance: u16, opcode: u16, n: usize, input: &[u8], out: &mut [u8], want: usize) -> Result<usize> {
        self.band_ok(n)?;
        let r = self.exact(
            &Call { channel, client: abi::CLIENT_BAND, instance, opcode, arg1: n as u16, body: input, ..Call::default() },
            out,
            want.min(out.len()),
        )?;
        Ok(r.stored(out.len()))
    }

    /// `n` narrowband samples per frame (80 for 10 ms).  In: 8n bytes of
    /// 32 kHz PCM.  Out: 14n bytes, three buffers: 2n (0 to 4 kHz at 8 kHz),
    /// 4n (the 4 to 8 kHz part, at 16 kHz), 8n (the part above 8 kHz, at
    /// 32 kHz).
    pub fn band_split_swb(&mut self, channel: u16, instance: u16, n: usize, input: &[u8], out: &mut [u8]) -> Result<usize> {
        self.band_job(channel, instance, band::SPLIT_SWB, n, &input[..8 * n], out, 14 * n)
    }

    /// In: 4n bytes of 16 kHz PCM.  Out: 6n bytes, two buffers.
    pub fn band_split_wb(&mut self, channel: u16, instance: u16, n: usize, input: &[u8], out: &mut [u8]) -> Result<usize> {
        self.band_job(channel, instance, band::SPLIT_WB, n, &input[..4 * n], out, 6 * n)
    }

    /// In: the three band buffers (14n bytes).  Out: 14n bytes again; the
    /// 32 kHz signal is in the third buffer and the 16 kHz signal in the
    /// second (read off the traces: both hold full-band audio in results).
    pub fn band_combine_swb(&mut self, channel: u16, instance: u16, n: usize, input: &[u8], out: &mut [u8]) -> Result<usize> {
        self.band_job(channel, instance, band::COMBINE_SWB, n, &input[..14 * n], out, 14 * n)
    }

    /// In: two band buffers (6n bytes).  Out: 6n bytes again; the 16 kHz
    /// signal is in the second buffer (it is what the encoder is then given).
    pub fn band_combine_wb(&mut self, channel: u16, instance: u16, n: usize, input: &[u8], out: &mut [u8]) -> Result<usize> {
        self.band_job(channel, instance, band::COMBINE_WB, n, &input[..6 * n], out, 6 * n)
    }
}
