// SPDX-License-Identifier: GPL-2.0-or-later
//! The two things the audio engine drives: the audio front end and the DSP.
//! Both are traits so that the engine runs unchanged against the phone's
//! devices (through the hardware layer and `dphone-vpm`) and against fakes.

use std::io;

use dphone_vpm::{Codec, Family};

use crate::bands::{BandFrame, WB};

pub type DspResult<T> = dphone_vpm::Result<T>;

/// Acoustic path.  Which PCM is open selects it; speaker and headset
/// exclude each other in the hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudioPath {
    Handset,
    Headset,
    Speaker,
}

impl AudioPath {
    pub const ALL: [AudioPath; 3] = [AudioPath::Handset, AudioPath::Headset, AudioPath::Speaker];

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Front-end gain controls the engine passes through (they are mixer
/// controls of the sound card; the values are the control's own units).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GainControl {
    /// Microphone (ADC) gain.
    Mic,
    /// Output (DAC) gain.
    Output,
    /// Sidetone, done by the front end itself.
    Sidetone,
}

/// The audio front end: one capture and one playback stream of 16-bit mono
/// at the family's rate (32 kHz new, 16 kHz old).
pub trait AudioDevice: Send {
    fn rate(&self) -> u32;
    /// Opens capture and playback on a path, closing any other.
    fn open(&mut self, path: AudioPath) -> io::Result<()>;
    fn close(&mut self);
    /// Blocks until `buf` (10 ms) is filled.  This is the engine's clock.
    fn read(&mut self, buf: &mut [i16]) -> io::Result<()>;
    /// Queues 10 ms for playback; must not block for long.
    fn write(&mut self, buf: &[i16]) -> io::Result<()>;
    fn set_gain(&mut self, path: AudioPath, control: GainControl, value: i32) -> io::Result<()>;
}

/// The DSP as the engine uses it.  All audio is 10 ms.  The device side
/// works on a path, the stream side on a voice stream slot (0 to 2).
pub trait Dsp: Send {
    fn family(&self) -> Family;
    /// Sends the one-time start-up configuration.  The DSP itself must
    /// already be running.  `force` repeats it (after a DSP restart).
    fn start(&mut self, force: bool) -> DspResult<()>;

    /// What the last start-up did, for the log (jobs sent, and the first
    /// failure if any).
    fn startup_summary(&self) -> Option<String> {
        None
    }

    /// A path's canceller starts being used.  `rx_gain_q8` is the receive
    /// gain in dB * 256; `tone` is as for [`Dsp::set_tone_mode`].
    fn attach(&mut self, path: AudioPath, rx_gain_q8: i16, tone: bool) -> DspResult<()>;
    fn detach(&mut self, path: AudioPath) -> DspResult<()>;
    fn set_rx_gain(&mut self, path: AudioPath, rx_gain_q8: i16) -> DspResult<()>;
    /// A local tone (dial tone, ringback, a key tone, ...) started or
    /// stopped playing on the attached path.  The stock firmware tells the
    /// path's canceller ("tone mode").
    fn set_tone_mode(&mut self, path: AudioPath, tone: bool) -> DspResult<()>;

    /// Microphone PCM (10 ms at the device rate) to band buffers.
    fn mic_to_bands(&mut self, path: AudioPath, pcm: &[i16], out: &mut BandFrame) -> DspResult<()>;
    /// Band buffers to loudspeaker or receiver PCM.
    fn bands_to_speaker(&mut self, path: AudioPath, bands: &BandFrame, pcm: &mut [i16]) -> DspResult<()>;
    /// The canceller's microphone-side job, in place: echo removed.
    fn ingress(&mut self, path: AudioPath, frame: &mut BandFrame) -> DspResult<()>;
    /// The canceller's receive-side job, in place: it takes the far-end
    /// signal as its reference and returns what is to be played.
    fn egress(&mut self, path: AudioPath, frame: &mut BandFrame) -> DspResult<()>;
    /// Per-tick housekeeping the stock firmware does (state poll, and
    /// statistics once a second).
    fn poll(&mut self, path: AudioPath) -> DspResult<()>;

    /// 16 kHz PCM to band buffers on a stream's filter bank.
    fn wb_to_bands(&mut self, slot: usize, wb: &[i16; WB], out: &mut BandFrame) -> DspResult<()>;
    /// Band buffers to 16 kHz PCM.
    fn bands_to_wb(&mut self, slot: usize, bands: &BandFrame, wb: &mut [i16; WB]) -> DspResult<()>;

    /// Prepares a stream slot's encoder and decoder.
    fn codec_open(&mut self, slot: usize, codec: Codec) -> DspResult<()>;
    /// One codec frame of PCM in, payload bytes in RTP order out.
    fn encode(&mut self, slot: usize, pcm: &[i16], out: &mut [u8]) -> DspResult<usize>;
    /// One codec frame of payload in, PCM out.
    fn decode(&mut self, slot: usize, frame: &[u8], pcm: &mut [i16]) -> DspResult<usize>;
    /// A frame was lost: produce what to play instead (silence, unless the
    /// DSP's own concealment is enabled).
    fn decode_lost(&mut self, slot: usize, pcm: &mut [i16]) -> DspResult<usize>;
}
