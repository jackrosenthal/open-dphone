// SPDX-License-Identifier: GPL-2.0-or-later
//! [`Dsp`] on the real job interface.
//!
//! # Signal routing: what is established and what is inferred
//!
//! Established from captured job bodies (D60 and D40, G.722 handset and
//! speakerphone calls; identical buffers followed from job to job, and the
//! spectrum of each buffer measured):
//!
//! - The band buffers are: below 4 kHz at 8 kHz; the 4 to 8 kHz part at
//!   16 kHz; (new family) the part above 8 kHz at 32 kHz.
//! - New family, microphone: 32 kHz PCM goes to the device's BAND split
//!   (op 1); its three output buffers are the canceller's ingress request;
//!   buffer 0 of the ingress result is buffer 0 of the stream's BAND
//!   combine-wideband request (op 5), and buffer 1 of that job's result is
//!   the encoder's input.
//! - New family, receive: the decoder's output is the stream's BAND
//!   split-wideband request (op 2); buffer 0 of the canceller's egress
//!   result is buffer 0 of the device's BAND combine request (op 4), whose
//!   result holds the 32 kHz signal in its third buffer.
//! - Combine jobs return a frame of the request's size with the combined
//!   signals in the upper buffers (16 kHz in the second, 32 kHz in the
//!   third).
//! - The speakerphone canceller takes two buffers (480 bytes) on both
//!   families; the handset canceller three on the new family, two on the
//!   old.  Handset is HEC instance 0, speakerphone the SBAEC.
//! - Narrowband codecs and G.711 use buffer 0 only, with no stream-side
//!   BAND job.
//! - Tone mode (vendor module `ept_core`, and registers read back from a
//!   stock D60): while a path plays a local tone the vendor sets option
//!   bits in its canceller (HEC register 0 = 0x8002; SBAEC register 0
//!   bit 1 and register 330 bit 15, 329 on the old family) and clears
//!   them when the tone stops or the path is left.
//! - The speakerphone canceller's frame jobs take argument 1 = 80 with
//!   the 480-byte frame, and return the whole frame while bit 0 of the
//!   direction's flag register is set (the start-up sequence sets both).
//! - Old family, per 10 ms of a G.722 speakerphone call, in order: decode,
//!   canceller egress, canceller ingress, encode.
//!
//! Inferred (see the experiments in the crate documentation):
//!
//! - Buffer 1 of the ingress result feeds buffer 1 of the stream combine
//!   (only buffer 0 could be followed; the request's own buffer 1 is
//!   overwritten by the result before a trace sees it).
//! - What the stream's split produces goes into the egress request
//!   unchanged except for mixing (the buffers differ in the traces, which
//!   fits mixing or gain but was not resolved).
//! - The order of jobs within a tick on the new family.
//! - The headset uses HEC instance 1.
//! - Receive gain: HEC register 4 and SBAEC register 2, in dB * 256
//!   (the SBAEC's follows the volume on stock, read back from a D60).
//! - The old family's ARM-side filter bank (the vendor's filters were not
//!   recovered; ours has the same structure).
//! - G.726's bytes are pair-swapped like G.722's.

use dphone_vpm::replay::JobLike;
use dphone_vpm::startup::{self, StartupOptions, Tuning};
use dphone_vpm::transport::Transport;
use dphone_vpm::vpm::{hec, sbaec};
use dphone_vpm::{ChannelMap, Codec, CodecParams, Error, Family, Vpm, codec::swap_pairs};

use crate::bands::{BandFrame, Combine2, NB, SWB, Split2, WB};
use crate::device::{AudioPath, Dsp, DspResult};

pub const SLOTS: usize = dphone_vpm::channels::NUM_STREAMS;

#[derive(Clone, Copy, Debug)]
pub struct VpmDspOptions {
    /// Send the codec queries at start-up, as the stock firmware does.
    pub queries: bool,
    /// Poll the canceller's state block every tick, as the stock firmware
    /// does (the reply is not used).
    pub poll_state: bool,
    /// Ask the DSP to conceal a lost frame with a decode job of frame type
    /// 6 and no data.  That request was seen once in a trace and its
    /// meaning is a guess, so this is off: a lost frame plays as silence.
    pub plc_frame_type_6: bool,
}

impl Default for VpmDspOptions {
    fn default() -> Self {
        Self { queries: true, poll_state: true, plc_frame_type_6: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Canceller {
    Hec(u16),
    Sbaec,
}

fn canceller(path: AudioPath) -> Canceller {
    match path {
        AudioPath::Handset => Canceller::Hec(0),
        AudioPath::Headset => Canceller::Hec(1),
        AudioPath::Speaker => Canceller::Sbaec,
    }
}

pub struct VpmDsp<T: Transport> {
    vpm: Vpm<T>,
    map: ChannelMap,
    opts: VpmDspOptions,
    tuning: Tuning,
    started: bool,
    startup: Option<String>,
    gains: [i16; 3],
    codecs: [Option<CodecParams>; SLOTS],
    // old family: the filter bank runs here
    dev_split: Split2,
    dev_combine: Combine2,
    stream_split: [Split2; SLOTS],
    stream_combine: [Combine2; SLOTS],
    ticks: u32,
    frame: [u8; hec::FRAME_NEW],
    out: [u8; hec::FRAME_NEW + 8],
    /// The last state block and statistics, for diagnostics.
    pub last_state: [u8; hec::STATE_BYTES],
    pub last_stats: (i16, i16),
}

impl<T: Transport + Send> VpmDsp<T> {
    pub fn new(vpm: Vpm<T>, opts: VpmDspOptions) -> Self {
        let family = vpm.family();
        let tuning = startup::tuning(family);
        let g = tuning.gains;
        Self {
            map: ChannelMap::new(family),
            opts,
            gains: [g.handset, g.headset, g.speaker],
            tuning,
            started: false,
            startup: None,
            codecs: [None; SLOTS],
            dev_split: Split2::new(),
            dev_combine: Combine2::new(),
            stream_split: [const { Split2::new() }; SLOTS],
            stream_combine: [const { Combine2::new() }; SLOTS],
            ticks: 0,
            frame: [0; hec::FRAME_NEW],
            out: [0; hec::FRAME_NEW + 8],
            last_state: [0; hec::STATE_BYTES],
            last_stats: (0, 0),
            vpm,
        }
    }

    pub fn vpm(&mut self) -> &mut Vpm<T> {
        &mut self.vpm
    }

    pub fn tuning(&self) -> &Tuning {
        &self.tuning
    }

    fn new_family(&self) -> bool {
        self.vpm.family() == Family::New
    }

    fn slot(slot: usize) -> DspResult<usize> {
        if slot < SLOTS { Ok(slot) } else { Err(Error::Invalid) }
    }

    fn init_device_band(&mut self, path: AudioPath) -> DspResult<()> {
        let band = match canceller(path) {
            Canceller::Hec(i) => self.map.hec_band(i),
            Canceller::Sbaec => self.map.sbaec_band(),
        };
        if let Some(b) = band {
            self.vpm.band_split_init(b.split, b.instance)?;
            self.vpm.band_combine_init(b.combine, b.instance)?;
        } else {
            self.dev_split.reset();
            self.dev_combine.reset();
        }
        Ok(())
    }

    /// Tone mode of a path's canceller, as the stock firmware sets it.
    fn tone_mode(&mut self, path: AudioPath, tone: bool) -> DspResult<()> {
        match canceller(path) {
            Canceller::Sbaec => {
                let sb = self.map.sbaec();
                let reg = sbaec::tone_reg(self.vpm.family());
                let with = |file: u16, bits: u16| if tone { file | bits } else { file & !bits };
                let options = with(self.tuning.sbaec[usize::from(sbaec::REG_OPTIONS)], sbaec::OPTIONS_TONE);
                let second = with(self.tuning.sbaec[usize::from(reg)], sbaec::TONE_REG_BIT);
                self.vpm.sbaec_set_reg(sb, 0, sbaec::REG_OPTIONS, options)?;
                self.vpm.sbaec_set_reg(sb, 0, reg, second)
            }
            Canceller::Hec(i) => {
                let file = self.tuning.hec[usize::from(i)][usize::from(hec::REG_OPTIONS)];
                let options = if tone { file | hec::OPTIONS_TONE } else { file & !hec::OPTIONS_TONE };
                self.vpm.hec_set_reg(self.map.hec(i), i, hec::REG_OPTIONS, options)
            }
        }
    }
}

fn pcm_to_bytes(pcm: &[i16], out: &mut [u8]) {
    for (b, s) in out.as_chunks_mut::<2>().0.iter_mut().zip(pcm) {
        b.copy_from_slice(&s.to_le_bytes());
    }
}

fn bytes_to_pcm(bytes: &[u8], out: &mut [i16]) {
    for (s, b) in out.iter_mut().zip(bytes.as_chunks::<2>().0) {
        *s = i16::from_le_bytes([b[0], b[1]]);
    }
}

impl<T: Transport + Send> Dsp for VpmDsp<T> {
    fn family(&self) -> Family {
        self.vpm.family()
    }

    fn start(&mut self, force: bool) -> DspResult<()> {
        if self.started && !force {
            return Ok(());
        }
        self.started = false;
        self.codecs = [None; SLOTS];
        let opts = StartupOptions { queries: self.opts.queries, keep_going: false };
        let mut sent = 0u32;
        let mut failure = None;
        let mut notes = String::new();
        let result = startup::run_startup(&mut self.vpm, &opts, |job, r| match r {
            Ok(reply) => {
                sent += 1;
                // Canceller jobs answer with two values; anything but the
                // request's own arguments is worth seeing.
                if (job.client == dphone_vpm::abi::CLIENT_SBAEC || job.client == dphone_vpm::abi::CLIENT_HEC)
                    && (reply.value1 != job.arg1 || reply.value2 != job.arg2)
                {
                    notes.push_str(&format!(
                        "; client {} operation {} answered {} {} to {} {}",
                        job.client, job.op, reply.value1, reply.value2, job.arg1, job.arg2
                    ));
                }
            }
            Err(e) if failure.is_none() => failure = Some(format!("{}: {e}", job.describe())),
            Err(_) => {}
        });
        self.startup = Some(match &failure {
            None => format!("{sent} start-up jobs accepted{notes}"),
            Some(f) => format!("{sent} start-up jobs accepted, then {f}"),
        });
        result?;
        self.started = true;
        Ok(())
    }

    fn startup_summary(&self) -> Option<String> {
        self.startup.clone()
    }

    fn attach(&mut self, path: AudioPath, rx_gain_q8: i16, tone: bool) -> DspResult<()> {
        self.gains[path.index()] = rx_gain_q8;
        // as the stock firmware does when a device is attached: the
        // filter-bank init pair, the receive gain, and the tone mode
        self.init_device_band(path)?;
        match canceller(path) {
            Canceller::Sbaec => self.vpm.sbaec_set_reg(self.map.sbaec(), 0, sbaec::REG_RX_GAIN, rx_gain_q8 as u16)?,
            Canceller::Hec(i) => self.vpm.hec_set_reg(self.map.hec(i), i, hec::REG_RX_GAIN, rx_gain_q8 as u16)?,
        }
        self.tone_mode(path, tone)
    }

    fn detach(&mut self, path: AudioPath) -> DspResult<()> {
        // a path that is left is out of tone mode
        self.tone_mode(path, false)
    }

    fn set_rx_gain(&mut self, path: AudioPath, rx_gain_q8: i16) -> DspResult<()> {
        self.gains[path.index()] = rx_gain_q8;
        match canceller(path) {
            Canceller::Sbaec => self.vpm.sbaec_set_reg(self.map.sbaec(), 0, sbaec::REG_RX_GAIN, rx_gain_q8 as u16),
            Canceller::Hec(i) => self.vpm.hec_set_reg(self.map.hec(i), i, hec::REG_RX_GAIN, rx_gain_q8 as u16),
        }
    }

    fn set_tone_mode(&mut self, path: AudioPath, tone: bool) -> DspResult<()> {
        self.tone_mode(path, tone)
    }

    fn mic_to_bands(&mut self, path: AudioPath, pcm: &[i16], out: &mut BandFrame) -> DspResult<()> {
        if self.new_family() {
            if pcm.len() != SWB {
                return Err(Error::Invalid);
            }
            let b = match canceller(path) {
                Canceller::Hec(i) => self.map.hec_band(i),
                Canceller::Sbaec => self.map.sbaec_band(),
            }
            .ok_or(Error::NotOnThisFamily)?;
            pcm_to_bytes(pcm, &mut self.frame[..2 * SWB]);
            self.vpm.band_split_swb(b.split, b.instance, NB, &self.frame[..2 * SWB], &mut self.out[..hec::FRAME_NEW])?;
            out.from_bytes(&self.out[..hec::FRAME_NEW]);
        } else {
            if pcm.len() != WB {
                return Err(Error::Invalid);
            }
            self.dev_split.run(pcm, &mut out.nb, &mut out.hi1);
            out.hi2 = [0; SWB];
        }
        Ok(())
    }

    fn bands_to_speaker(&mut self, path: AudioPath, bands: &BandFrame, pcm: &mut [i16]) -> DspResult<()> {
        if self.new_family() {
            if pcm.len() != SWB {
                return Err(Error::Invalid);
            }
            let b = match canceller(path) {
                Canceller::Hec(i) => self.map.hec_band(i),
                Canceller::Sbaec => self.map.sbaec_band(),
            }
            .ok_or(Error::NotOnThisFamily)?;
            bands.to_bytes(&mut self.frame[..hec::FRAME_NEW]);
            self.vpm.band_combine_swb(b.combine, b.instance, NB, &self.frame[..hec::FRAME_NEW], &mut self.out[..hec::FRAME_NEW])?;
            // the 32 kHz signal is the third buffer of the result
            bytes_to_pcm(&self.out[2 * (NB + WB)..hec::FRAME_NEW], pcm);
        } else {
            if pcm.len() != WB {
                return Err(Error::Invalid);
            }
            self.dev_combine.run(&bands.nb, &bands.hi1, pcm);
        }
        Ok(())
    }

    fn ingress(&mut self, path: AudioPath, frame: &mut BandFrame) -> DspResult<()> {
        match canceller(path) {
            Canceller::Hec(i) => {
                let n = self.vpm.hec_frame_bytes();
                frame.to_bytes(&mut self.frame[..n]);
                self.vpm.hec_ingress(self.map.hec(i), i, &mut self.frame[..n])?;
                frame.from_bytes(&self.frame[..n]);
            }
            Canceller::Sbaec => {
                frame.to_bytes(&mut self.frame[..sbaec::FRAME]);
                self.vpm.sbaec_ingress(self.map.sbaec(), 0, &mut self.frame[..sbaec::FRAME])?;
                frame.from_bytes(&self.frame[..sbaec::FRAME]);
                // the part above 8 kHz did not pass the canceller
                frame.hi2 = [0; SWB];
            }
        }
        Ok(())
    }

    fn egress(&mut self, path: AudioPath, frame: &mut BandFrame) -> DspResult<()> {
        match canceller(path) {
            Canceller::Hec(i) => {
                let n = self.vpm.hec_frame_bytes();
                frame.to_bytes(&mut self.frame[..n]);
                self.vpm.hec_egress(self.map.hec(i), i, &mut self.frame[..n])?;
                frame.from_bytes(&self.frame[..n]);
            }
            Canceller::Sbaec => {
                frame.to_bytes(&mut self.frame[..sbaec::FRAME]);
                self.vpm.sbaec_egress(self.map.sbaec(), 0, &mut self.frame[..sbaec::FRAME])?;
                frame.from_bytes(&self.frame[..sbaec::FRAME]);
            }
        }
        Ok(())
    }

    fn poll(&mut self, path: AudioPath) -> DspResult<()> {
        self.ticks = self.ticks.wrapping_add(1);
        let second = self.ticks.is_multiple_of(100);
        match canceller(path) {
            Canceller::Hec(i) => {
                let ch = self.map.hec(i);
                if self.opts.poll_state {
                    self.vpm.hec_get_state(ch, i, &mut self.last_state)?;
                }
                if second {
                    let mut stats = [0u8; 8];
                    self.vpm.hec_get_stats(ch, i, &mut stats)?;
                }
            }
            Canceller::Sbaec => {
                let ch = self.map.sbaec();
                if self.opts.poll_state {
                    self.vpm.sbaec_get_state(ch, 0, &mut self.last_state[..sbaec::STATE_BYTES])?;
                }
                if second {
                    self.last_stats = self.vpm.sbaec_get_stats(ch, 0)?;
                }
            }
        }
        Ok(())
    }

    fn wb_to_bands(&mut self, slot: usize, wb: &[i16; WB], out: &mut BandFrame) -> DspResult<()> {
        let slot = Self::slot(slot)?;
        if let Some(b) = self.map.stream_band(slot) {
            pcm_to_bytes(wb, &mut self.frame[..2 * WB]);
            self.vpm.band_split_wb(b.split, b.instance, NB, &self.frame[..2 * WB], &mut self.out[..sbaec::FRAME])?;
            out.from_bytes(&self.out[..sbaec::FRAME]);
        } else {
            self.stream_split[slot].run(wb, &mut out.nb, &mut out.hi1);
        }
        out.hi2 = [0; SWB];
        Ok(())
    }

    fn bands_to_wb(&mut self, slot: usize, bands: &BandFrame, wb: &mut [i16; WB]) -> DspResult<()> {
        let slot = Self::slot(slot)?;
        if let Some(b) = self.map.stream_band(slot) {
            bands.to_bytes(&mut self.frame[..sbaec::FRAME]);
            self.vpm.band_combine_wb(b.combine, b.instance, NB, &self.frame[..sbaec::FRAME], &mut self.out[..sbaec::FRAME])?;
            // the 16 kHz signal is the second buffer of the result
            bytes_to_pcm(&self.out[2 * NB..sbaec::FRAME], wb);
        } else {
            self.stream_combine[slot].run(&bands.nb, &bands.hi1, wb);
        }
        Ok(())
    }

    fn codec_open(&mut self, slot: usize, codec: Codec) -> DspResult<()> {
        let slot = Self::slot(slot)?;
        let p = codec.params(self.vpm.family())?;
        let (enc, dec) = self.map.pve(slot);
        let inst = slot as u16;
        self.codecs[slot] = None;
        // the order the stock firmware uses at call setup
        let band = if p.pcm_rate == 16_000 { self.map.stream_band(slot) } else { None };
        if let Some(b) = band {
            self.vpm.band_split_init(b.split, b.instance)?;
        }
        self.vpm.pve_dec_init(dec, inst, p.number, &p.dec_cfg)?;
        if let Some(b) = band {
            self.vpm.band_combine_init(b.combine, b.instance)?;
        }
        self.vpm.pve_enc_init(enc, inst, p.number, &p.enc_cfg)?;
        self.stream_split[slot].reset();
        self.stream_combine[slot].reset();
        self.codecs[slot] = Some(p);
        Ok(())
    }

    fn encode(&mut self, slot: usize, pcm: &[i16], out: &mut [u8]) -> DspResult<usize> {
        let slot = Self::slot(slot)?;
        let p = self.codecs[slot].ok_or(Error::Invalid)?;
        if pcm.len() != p.frame_samples || out.len() < p.frame_bytes {
            return Err(Error::Invalid);
        }
        let (enc, _) = self.map.pve(slot);
        pcm_to_bytes(pcm, &mut self.frame[..p.pcm_bytes()]);
        let (n, _status) = self.vpm.pve_encode(enc, slot as u16, p.number, 0, &self.frame[..p.pcm_bytes()], &mut self.out[..512])?;
        if n != p.frame_bytes {
            return Err(Error::ShortReply { want: p.frame_bytes, got: n, client: 1, channel: enc, opcode: 0 });
        }
        out[..n].copy_from_slice(&self.out[..n]);
        swap_pairs(&mut out[..n]);
        Ok(n)
    }

    fn decode(&mut self, slot: usize, frame: &[u8], pcm: &mut [i16]) -> DspResult<usize> {
        let slot = Self::slot(slot)?;
        let p = self.codecs[slot].ok_or(Error::Invalid)?;
        if frame.len() != p.frame_bytes || pcm.len() < p.frame_samples {
            return Err(Error::Invalid);
        }
        let (_, dec) = self.map.pve(slot);
        self.frame[..frame.len()].copy_from_slice(frame);
        swap_pairs(&mut self.frame[..frame.len()]);
        let n = self.vpm.pve_decode(dec, slot as u16, p.number, 1, &self.frame[..frame.len()], &mut self.out[..2 * WB])?;
        if n != p.pcm_bytes() {
            return Err(Error::ShortReply { want: p.pcm_bytes(), got: n, client: 1, channel: dec, opcode: 1 });
        }
        bytes_to_pcm(&self.out[..n], pcm);
        Ok(p.frame_samples)
    }

    fn decode_lost(&mut self, slot: usize, pcm: &mut [i16]) -> DspResult<usize> {
        let slot = Self::slot(slot)?;
        let p = self.codecs[slot].ok_or(Error::Invalid)?;
        if pcm.len() < p.frame_samples {
            return Err(Error::Invalid);
        }
        pcm[..p.frame_samples].fill(0);
        if self.opts.plc_frame_type_6 {
            let (_, dec) = self.map.pve(slot);
            let n = self.vpm.pve_decode(dec, slot as u16, p.number, 6, &[], &mut self.out[..2 * WB])?;
            if n == p.pcm_bytes() {
                bytes_to_pcm(&self.out[..n], pcm);
            }
        }
        Ok(p.frame_samples)
    }
}
