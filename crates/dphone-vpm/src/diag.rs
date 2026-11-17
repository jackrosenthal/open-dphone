// SPDX-License-Identifier: GPL-2.0-or-later
//! Diagnostics for the `dphone` debugging subcommands: a hardware-free
//! self-test, the G.722 loop, codec queries, replay, and single-job probes
//! of the cancellers and the filter bank.

use crate::codec::{Codec, swap_pairs};
use crate::replay::{self, JobLike};
use crate::startup::{self, StartupOptions};
use crate::transport::{MockTransport, Transport};
use crate::vpm::{Vpm, hec, pve, sbaec};
use crate::{ChannelMap, Error, Family, Result, abi};

#[derive(Clone, Debug, Default)]
pub struct SelfTestReport {
    pub checks: u32,
    pub failures: Vec<String>,
}

impl SelfTestReport {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        self.checks += 1;
        if !ok {
            self.failures.push(what());
        }
    }
}

/// Hardware-free checks: the ABI's sizes and ioctl numbers, the request
/// each wrapper builds (against frames seen on stock phones), and the
/// compiled-in replay tables.  The ring rules are the kernel driver's and
/// are tested with it, not here.
pub fn selftest() -> SelfTestReport {
    let mut r = SelfTestReport::default();
    r.check(size_of::<abi::Info>() == 64, || "struct bcmring_vpm_info is not 64 bytes".into());
    r.check(size_of::<abi::Job>() == 48, || "struct bcmring_vpm_job is not 48 bytes".into());
    r.check(abi::IOC_INFO == 0x8040_b900, || format!("info ioctl number {:#x}", abi::IOC_INFO));
    r.check(abi::IOC_JOB == 0xc030_b901, || format!("job ioctl number {:#x}", abi::IOC_JOB));

    for family in [Family::New, Family::Old] {
        let new = family == Family::New;
        let map = ChannelMap::new(family);
        let mut vpm = match Vpm::new(MockTransport::new(family)) {
            Ok(v) => v,
            Err(e) => {
                r.check(false, || format!("mock open: {e}"));
                continue;
            }
        };
        let g722 = Codec::G722.params(family).expect("G.722 is supported");
        let (enc, dec) = map.pve(1);
        let pcm = [0u8; 320];
        let mut out = [0u8; 1200];
        let _ = vpm.pve_enc_init(enc, 1, g722.number, &g722.enc_cfg);
        let _ = vpm.pve_dec_init(dec, 1, g722.number, &g722.dec_cfg);
        let _ = vpm.pve_encode(enc, 1, g722.number, 0, &pcm, &mut out);
        let _ = vpm.pve_decode(dec, 1, g722.number, 1, &[0u8; 80], &mut out);
        let mut frame = vec![0u8; vpm.hec_frame_bytes()];
        let _ = vpm.hec_ingress(map.hec(0), 0, &mut frame);
        let mut sframe = [0u8; sbaec::FRAME];
        let _ = vpm.sbaec_egress(map.sbaec(), 0, &mut sframe);
        let _ = vpm.sbaec_ingress(map.sbaec(), 0, &mut sframe);
        let regs = vec![0u16; vpm.sbaec_num_regs()];
        let _ = vpm.sbaec_set_regs(map.sbaec(), 0, 0, &regs);
        let _ = vpm.hec_set_reg(map.hec(0), 0, hec::REG_RX_GAIN, 0xf700);
        let band = vpm.band_split_swb(19, 5, 80, &[0u8; 640], &mut out);
        let jobs = std::mem::take(&mut vpm.transport().jobs);
        let fam = if new { "new" } else { "old" };

        let j = &jobs[0];
        r.check(
            (j.client, j.channel, j.instance, j.codec, j.opcode) == (1, if new { 4 } else { 2 }, 1, if new { 46 } else { 40 }, 0)
                && j.body == [0, 0, 0, 0, 1, 0],
            || format!("{fam}: G.722 encoder init request {j:?}"),
        );
        let j = &jobs[1];
        r.check(j.opcode == 3 && j.channel == if new { 5 } else { 3 } && j.body == [0, 0, 1, 0], || {
            format!("{fam}: G.722 decoder init request {j:?}")
        });
        let j = &jobs[2];
        r.check(j.opcode == 2 && (j.arg1, j.arg2) == (0, 0) && j.body.len() == 320, || format!("{fam}: encode request {j:?}"));
        let j = &jobs[3];
        let want = if new { (1, 80, pve::DEC_BUF_NEW) } else { (1, 0, pve::DEC_BUF_OLD) };
        r.check(j.opcode == 5 && (j.arg1, j.arg2, j.body.len()) == want, || format!("{fam}: decode request {j:?}"));
        let j = &jobs[4];
        r.check(
            (j.client, j.channel, j.opcode, j.body.len()) == (4, if new { 12 } else { 6 }, 2, if new { 1120 } else { 480 }),
            || format!("{fam}: HEC ingress request {j:?}"),
        );
        let j = &jobs[5];
        r.check((j.client, j.channel, j.opcode, j.arg1, j.body.len()) == (3, if new { 18 } else { 8 }, 8, 80, 480), || {
            format!("{fam}: SBAEC egress request {j:?}")
        });
        let j = &jobs[6];
        r.check((j.opcode, j.arg1, j.body.len()) == (9, 80, 480), || format!("{fam}: SBAEC ingress request {j:?}"));
        let j = &jobs[7];
        let last = if new { 0x3a3 } else { 0x3a2 };
        r.check((j.opcode, j.arg1, j.arg2, j.body.len()) == (5, 0, last, 2 * (usize::from(last) + 1)), || {
            format!("{fam}: SBAEC set-registers request {j:?}")
        });
        let j = &jobs[8];
        r.check((j.client, j.opcode, j.arg1, j.arg2) == (4, 6, 4, 0xf700), || format!("{fam}: HEC set-register request {j:?}"));
        if new {
            let j = &jobs[9];
            r.check(band == Ok(1120) && (j.client, j.opcode, j.arg1, j.body.len()) == (8, 1, 80, 640), || {
                format!("new: band split request {j:?} -> {band:?}")
            });
        } else {
            r.check(band == Err(Error::NotOnThisFamily), || "old: the BAND client must be refused".into());
        }

        // replay tables: frame lengths as the traces recorded them
        for seq in replay::table(family).sequences {
            for j in seq.jobs {
                let ok = replay::header_words(j.client).is_some_and(|h| usize::from(j.frame_words) == usize::from(h) + j.body.len());
                r.check(ok, || format!("{fam}: {} {}: frame length", seq.name, j.describe()));
            }
        }
        // the start-up sequence goes through
        let mut vpm = Vpm::new(MockTransport::new(family)).expect("mock");
        let res = startup::run_startup(&mut vpm, &StartupOptions::default(), |_, _| {});
        r.check(matches!(&res, Ok(rep) if rep.failed == 0), || format!("{fam}: start-up against the mock: {res:?}"));
    }
    r
}

/// The firmware's codec list (PVE op 7): pairs of (codec number, index).
pub fn query_codec_list<T: Transport>(vpm: &mut Vpm<T>) -> Result<Vec<(u16, u16)>> {
    let map = ChannelMap::new(vpm.family());
    let mut buf = [0u8; 1024];
    let (count, len) = vpm.pve_query_rate_codes(map.pve(0).1, 0, &mut buf)?;
    Ok(buf[..len.min(4 * usize::from(count))]
        .as_chunks::<4>().0.iter()
        .map(|c| (u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]])))
        .collect())
}

/// One codec's properties (PVE op 6) as 16-bit words.
pub fn query_codec<T: Transport>(vpm: &mut Vpm<T>, codec: u16) -> Result<Vec<u16>> {
    let map = ChannelMap::new(vpm.family());
    let mut buf = [0u8; pve::CODEC_PROPS];
    let len = vpm.pve_query_codec(map.pve(0).1, 0, codec, &mut buf)?;
    Ok(buf[..len].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
}

#[derive(Clone, Copy, Debug)]
pub struct G722LoopOptions {
    /// Voice stream whose channels and instance are used.  Stream 2 is the
    /// one a running phone is least likely to be using.
    pub stream: usize,
    pub send_init: bool,
    pub encode_only: bool,
}

impl Default for G722LoopOptions {
    fn default() -> Self {
        Self { stream: 2, send_init: true, encode_only: false }
    }
}

#[derive(Clone, Debug, Default)]
pub struct G722LoopResult {
    /// Decoded audio, 16 kHz.
    pub pcm: Vec<i16>,
    /// The encoded stream in standard (RTP) byte order.
    pub bits: Vec<u8>,
    pub frames: u32,
    /// Encode results whose status was not 1.
    pub bad_status: u32,
}

/// Encodes 16 kHz PCM with the DSP's G.722 encoder and decodes the result
/// with its decoder: 10 ms frames, 160 samples against 80 bytes.  A trailing
/// partial frame is padded with silence.  On stock DSPs the stream is
/// bit-identical to the standard codec's.
pub fn g722_loop<T: Transport>(vpm: &mut Vpm<T>, input: &[i16], opts: &G722LoopOptions) -> Result<G722LoopResult> {
    let p = Codec::G722.params(vpm.family())?;
    let (enc_ch, dec_ch) = ChannelMap::new(vpm.family()).pve(opts.stream);
    let inst = opts.stream as u16;
    if opts.send_init {
        vpm.pve_enc_init(enc_ch, inst, p.number, &p.enc_cfg)?;
        if !opts.encode_only {
            vpm.pve_dec_init(dec_ch, inst, p.number, &p.dec_cfg)?;
        }
    }
    let mut res = G722LoopResult::default();
    let mut pcm = [0u8; 320];
    let mut enc = [0u8; pve::DEC_BUF_NEW];
    let mut dec = [0u8; 640];
    for chunk in input.chunks(p.frame_samples) {
        pcm.fill(0);
        for (i, s) in chunk.iter().enumerate() {
            pcm[2 * i..2 * i + 2].copy_from_slice(&s.to_le_bytes());
        }
        let (n, status) = vpm.pve_encode(enc_ch, inst, p.number, 0, &pcm, &mut enc)?;
        if status != 1 {
            res.bad_status += 1;
        }
        let mut wire = enc[..n].to_vec();
        swap_pairs(&mut wire);
        res.bits.extend_from_slice(&wire);
        if !opts.encode_only {
            let m = vpm.pve_decode(dec_ch, inst, p.number, 1, &enc[..n], &mut dec)?;
            res.pcm.extend(dec[..m].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes([c[0], c[1]])));
        }
        res.frames += 1;
    }
    Ok(res)
}

/// Which canceller a probe addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Canceller {
    Hec(u16),
    Sbaec,
}

/// One canceller frame job with the caller's band buffers, for working out
/// what the canceller does to each (see the routing notes in
/// `dphone-media`).  `ingress` selects the microphone-side job.  The frame
/// is the family's size for that canceller and is replaced by the result.
pub fn canceller_frame<T: Transport>(vpm: &mut Vpm<T>, which: Canceller, ingress: bool, frame: &mut [u8]) -> Result<()> {
    let map = ChannelMap::new(vpm.family());
    match (which, ingress) {
        (Canceller::Hec(i), true) => vpm.hec_ingress(map.hec(i), i, frame),
        (Canceller::Hec(i), false) => vpm.hec_egress(map.hec(i), i, frame),
        (Canceller::Sbaec, true) => vpm.sbaec_ingress(map.sbaec(), 0, frame),
        (Canceller::Sbaec, false) => vpm.sbaec_egress(map.sbaec(), 0, frame),
    }
}

/// New family: split 10 ms of 32 kHz PCM (320 samples) with a stream's
/// filter bank and combine the result again.  Returns the three band
/// buffers (80, 160, 320 samples) and the combine job's whole result.
pub fn band_round_trip<T: Transport>(vpm: &mut Vpm<T>, stream: usize, pcm: &[i16; 320]) -> Result<(Vec<i16>, Vec<i16>)> {
    let b = ChannelMap::new(vpm.family()).stream_band(stream).ok_or(Error::NotOnThisFamily)?;
    let input: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut bands = [0u8; 1120];
    vpm.band_split_swb(b.split, b.instance, 80, &input, &mut bands)?;
    let mut out = [0u8; 1120];
    let n = vpm.band_combine_swb(b.combine, b.instance, 80, &bands, &mut out)?;
    let words = |b: &[u8]| b.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>();
    Ok((words(&bands), words(&out[..n])))
}

/// A test signal for the routing experiments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// A sine of this frequency in hertz.
    Tone(u32),
    /// A sweep from 100 Hz to just under half the sampling rate, once over
    /// the whole run.
    Sweep,
}

/// Peak of the test signals, of 32767.
pub const SIGNAL_PEAK: i16 = 8000;
/// Frames at the start of an experiment that the level figures leave out,
/// for the filters to settle.
pub const SETTLE_FRAMES: usize = 2;
/// Sampling rates of the three band buffers of a 10 ms frame (80, 160, and
/// 320 samples).
pub const BUFFER_RATES: [u32; 3] = [8000, 16000, 32000];

fn signal(sig: Signal, rate: u32, samples: usize) -> Vec<i16> {
    let mut phase = 0.0f64;
    (0..samples)
        .map(|i| {
            let hz = match sig {
                Signal::Tone(hz) => f64::from(hz),
                Signal::Sweep => 100.0 + (f64::from(rate) * 0.47 - 100.0) * i as f64 / samples as f64,
            };
            phase += std::f64::consts::TAU * hz / f64::from(rate);
            (f64::from(SIGNAL_PEAK) * phase.sin()) as i16
        })
        .collect()
}

/// Level figures of one buffer over a run of frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Level {
    sum_sq: u64,
    samples: u64,
    peak: u16,
    crossings: u64,
    last: i16,
}

impl Level {
    fn add(&mut self, samples: &[i16]) {
        for &s in samples {
            self.sum_sq += u64::from(s.unsigned_abs()) * u64::from(s.unsigned_abs());
            self.peak = self.peak.max(s.unsigned_abs());
            if (s >= 0) != (self.last >= 0) {
                self.crossings += 1;
            }
            self.last = s;
        }
        self.samples += samples.len() as u64;
    }

    /// Root mean square, of 32767.
    pub fn rms(&self) -> u32 {
        self.sum_sq.checked_div(self.samples).map_or(0, |mean| mean.isqrt() as u32)
    }

    pub fn peak(&self) -> u16 {
        self.peak
    }

    /// The frequency of a single tone with this many zero crossings, for a
    /// buffer sampled at `rate`.  Meaningless for silence and noise.
    pub fn crossing_hz(&self, rate: u32) -> u32 {
        (self.crossings * u64::from(rate)).checked_div(2 * self.samples).unwrap_or(0) as u32
    }

    fn describe(&self, rate: u32) -> String {
        format!("rms {:5} peak {:5}, {:5} Hz by zero crossings", self.rms(), self.peak(), self.crossing_hz(rate))
    }
}

fn words(bytes: &[u8]) -> Vec<i16> {
    bytes.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect()
}

/// The three band buffers of a frame of 16-bit words: 80, 160, and (when
/// the frame has it) 320 samples.
fn buffers(frame: &[i16]) -> Vec<&[i16]> {
    let mut out = Vec::new();
    let mut at = 0;
    for len in [80, 160, 320] {
        if at + len <= frame.len() {
            out.push(&frame[at..at + len]);
            at += len;
        }
    }
    out
}

/// What [`band_experiment`] measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BandReport {
    pub stream: usize,
    pub signal: Signal,
    pub frames: usize,
    /// Levels of the three buffers the split job returned.
    pub split: [Level; 3],
    /// Levels of the buffers the combine job returned (three when it
    /// returns a whole frame).
    pub combined: Vec<Level>,
    /// The delay in samples (0 to 640) at which the combine result's third
    /// buffer matches the input best, and the normalized correlation there
    /// in percent.  A tone matches at every multiple of its period, so
    /// only the sweep gives a delay that means something.
    pub delay: Option<(usize, i32)>,
}

impl std::fmt::Display for BandReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.signal {
            Signal::Tone(hz) => format!("a {hz} Hz tone"),
            Signal::Sweep => "a sweep from 100 Hz to 15 kHz".to_string(),
        };
        writeln!(
            f,
            "filter bank of stream {}: {} frames (10 ms each) of {what} at 32 kHz, peak {SIGNAL_PEAK}, split and combined again",
            self.stream, self.frames
        )?;
        writeln!(f, "levels are of 32767, over frames {} to {}", SETTLE_FRAMES + 1, self.frames)?;
        for (i, l) in self.split.iter().enumerate() {
            writeln!(f, "  split result, buffer {i} ({:2} kHz):   {}", BUFFER_RATES[i] / 1000, l.describe(BUFFER_RATES[i]))?;
        }
        for (i, l) in self.combined.iter().enumerate() {
            writeln!(f, "  combine result, buffer {i} ({:2} kHz): {}", BUFFER_RATES[i] / 1000, l.describe(BUFFER_RATES[i]))?;
        }
        match self.delay {
            Some((d, c)) => writeln!(f, "  combine result, buffer 2 against the input: best match at a delay of {d} samples, correlation {c} %"),
            None => writeln!(f, "  the combine job returned no third buffer to compare with the input"),
        }
    }
}

/// Routing experiment 1 (see `dphone-media`): send `frames` frames of a
/// test signal through [`band_round_trip`] and measure every buffer that
/// comes back.  New family only.
pub fn band_experiment<T: Transport>(vpm: &mut Vpm<T>, stream: usize, sig: Signal, frames: usize) -> Result<BandReport> {
    let input = signal(sig, BUFFER_RATES[2], frames * 320);
    let mut report = BandReport { stream, signal: sig, frames, split: [Level::default(); 3], combined: Vec::new(), delay: None };
    let mut full = Vec::with_capacity(input.len());
    for (n, chunk) in input.as_chunks::<320>().0.iter().enumerate() {
        let (bands, out) = band_round_trip(vpm, stream, chunk)?;
        let out_bufs = buffers(&out);
        if let Some(third) = out_bufs.get(2) {
            full.extend_from_slice(third);
        }
        if n < SETTLE_FRAMES {
            continue;
        }
        for (level, buf) in report.split.iter_mut().zip(buffers(&bands)) {
            level.add(buf);
        }
        report.combined.resize(out_bufs.len(), Level::default());
        for (level, buf) in report.combined.iter_mut().zip(out_bufs) {
            level.add(buf);
        }
    }
    if full.len() == input.len() && input.len() > 2 * 640 {
        report.delay = best_delay(&input, &full, 640);
    }
    Ok(report)
}

/// The delay of `out` against `input` (0 to `max` samples) with the largest
/// normalized correlation, and that correlation in percent.
fn best_delay(input: &[i16], out: &[i16], max: usize) -> Option<(usize, i32)> {
    let n = input.len().checked_sub(max)?;
    let energy = |s: &[i16]| s.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>();
    let e_in = energy(&input[..n]);
    let mut best: Option<(usize, f64)> = None;
    for d in 0..=max {
        let o = &out[d..d + n];
        let e_out = energy(o);
        if e_in == 0.0 || e_out == 0.0 {
            continue;
        }
        let dot: f64 = input[..n].iter().zip(o).map(|(&a, &b)| f64::from(a) * f64::from(b)).sum();
        let c = dot / (e_in * e_out).sqrt();
        if best.is_none_or(|(_, b)| c > b) {
            best = Some((d, c));
        }
    }
    best.map(|(d, c)| (d, (c * 100.0).round() as i32))
}

/// What [`canceller_experiment`] measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancellerReport {
    pub which: Canceller,
    pub ingress: bool,
    /// The buffer that carried the tone; the others were sent silent.
    pub buffer: usize,
    pub hz: u32,
    pub frames: usize,
    /// Per buffer of the frame: what was sent and what came back.
    pub levels: Vec<(Level, Level)>,
}

impl std::fmt::Display for CancellerReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let which = match self.which {
            Canceller::Hec(i) => format!("handset-type canceller {i}"),
            Canceller::Sbaec => "speakerphone canceller".to_string(),
        };
        writeln!(
            f,
            "{which}, {} jobs: {} frames (10 ms each) with a {} Hz tone, peak {SIGNAL_PEAK}, in buffer {} only",
            if self.ingress { "ingress (microphone side)" } else { "egress (receive side)" },
            self.frames,
            self.hz,
            self.buffer
        )?;
        writeln!(f, "levels are of 32767, over frames {} to {}", SETTLE_FRAMES + 1, self.frames)?;
        for (i, (sent, back)) in self.levels.iter().enumerate() {
            writeln!(f, "  buffer {i} ({:2} kHz): sent     {}", BUFFER_RATES[i] / 1000, sent.describe(BUFFER_RATES[i]))?;
            writeln!(f, "                    returned {}", back.describe(BUFFER_RATES[i]))?;
        }
        Ok(())
    }
}

/// Routing experiments 2 and 3 (see `dphone-media`): send `frames` frames
/// through [`canceller_frame`] with a tone in one band buffer and silence
/// in the others, and measure every buffer that comes back.  The canceller
/// is used as it stands: its registers (gain, options, the speakerphone
/// canceller's whole-frame flags) are whatever was set before.
pub fn canceller_experiment<T: Transport>(
    vpm: &mut Vpm<T>,
    which: Canceller,
    ingress: bool,
    buffer: usize,
    hz: u32,
    frames: usize,
) -> std::result::Result<CancellerReport, String> {
    let bytes = match which {
        Canceller::Hec(_) => vpm.hec_frame_bytes(),
        Canceller::Sbaec => sbaec::FRAME,
    };
    let lens: Vec<usize> = buffers(&vec![0i16; bytes / 2]).iter().map(|b| b.len()).collect();
    let Some(&len) = lens.get(buffer) else {
        return Err(format!("this canceller's frame has buffers 0 to {}", lens.len() - 1));
    };
    let rate = BUFFER_RATES[buffer];
    if hz == 0 || hz >= rate / 2 {
        return Err(format!("buffer {buffer} is sampled at {rate} Hz; the tone must be below {} Hz", rate / 2));
    }
    let start: usize = lens[..buffer].iter().sum();
    let tone = signal(Signal::Tone(hz), rate, frames * len);
    let mut report = CancellerReport { which, ingress, buffer, hz, frames, levels: vec![(Level::default(), Level::default()); lens.len()] };
    for (n, chunk) in tone.chunks_exact(len).enumerate() {
        let mut sent = vec![0i16; bytes / 2];
        sent[start..start + len].copy_from_slice(chunk);
        let mut frame: Vec<u8> = sent.iter().flat_map(|s| s.to_le_bytes()).collect();
        canceller_frame(vpm, which, ingress, &mut frame).map_err(|e| format!("frame {}: {e}", n + 1))?;
        if n < SETTLE_FRAMES {
            continue;
        }
        let back = words(&frame);
        for ((s, b), (sent_buf, back_buf)) in report.levels.iter_mut().zip(buffers(&sent).into_iter().zip(buffers(&back))) {
            s.add(sent_buf);
            b.add(back_buf);
        }
    }
    Ok(report)
}

/// One job with a free choice of header fields, reporting what came back:
/// the two result values and the result body's length.  For working out a
/// client's behavior on a phone.
#[allow(clippy::too_many_arguments)]
pub fn probe_job<T: Transport>(
    vpm: &mut Vpm<T>,
    client: u16,
    channel: u16,
    instance: u16,
    opcode: u16,
    arg1: u16,
    arg2: u16,
    body: &[u8],
) -> String {
    let mut reply = [0u8; 2048];
    let call = crate::vpm::Call { channel, client, instance, opcode, arg1, arg2, body, ..Default::default() };
    match vpm.job(&call, &mut reply) {
        Ok(r) => {
            let n = r.len.min(reply.len());
            let nonzero = reply[..n].iter().filter(|b| **b != 0).count();
            let words: Vec<String> =
                reply[..n].as_chunks::<2>().0.iter().map(|c| format!("{:04x}", u16::from_le_bytes(*c))).collect();
            format!("values {} {}, body {} bytes ({} nonzero)\n{}", r.value1, r.value2, r.len, nonzero, words.join(" "))
        }
        Err(e) => format!("error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selftest_passes() {
        let r = selftest();
        assert!(r.passed(), "{:#?}", r.failures);
        assert!(r.checks > 200);
    }

    #[test]
    fn g722_loop_sizes() {
        for family in [Family::New, Family::Old] {
            let mut vpm = Vpm::new(MockTransport::new(family)).unwrap();
            let input: Vec<i16> = (0..400).map(|i| (i * 50) as i16).collect();
            let r = g722_loop(&mut vpm, &input, &G722LoopOptions::default()).unwrap();
            assert_eq!(r.frames, 3);
            assert_eq!(r.bits.len(), 240);
            assert_eq!(r.pcm.len(), 480);
            assert_eq!(r.bad_status, 0);
            assert_eq!(query_codec_list(&mut vpm).unwrap().len(), if family == Family::New { 76 } else { 60 });
            assert_eq!(query_codec(&mut vpm, 4).unwrap().len(), 268);
        }
    }

    #[test]
    fn test_signals() {
        let tone = signal(Signal::Tone(1000), 16000, 1600);
        let mut l = Level::default();
        l.add(&tone);
        // A sine's RMS is its peak over the square root of two.
        assert!((5600..=5700).contains(&l.rms()), "{}", l.rms());
        assert!((7990..=8000).contains(&l.peak()));
        assert!((990..=1010).contains(&l.crossing_hz(16000)), "{}", l.crossing_hz(16000));
        assert_eq!(Level::default().rms(), 0);
        // A sweep delayed by 37 samples is found at that delay.
        let sweep = signal(Signal::Sweep, 32000, 6400);
        let mut delayed = vec![0i16; 37];
        delayed.extend_from_slice(&sweep[..6400 - 37]);
        assert_eq!(best_delay(&sweep, &delayed, 640), Some((37, 100)));
        assert_eq!(best_delay(&sweep, &vec![0; 6400], 640), None);
    }

    #[test]
    fn canceller_experiment_levels() {
        // The mock returns handset-type frames as sent.
        for (family, bufs) in [(Family::New, 3), (Family::Old, 2)] {
            let mut vpm = Vpm::new(MockTransport::new(family)).unwrap();
            let r = canceller_experiment(&mut vpm, Canceller::Hec(0), true, 1, 1000, 20).unwrap();
            assert_eq!(r.levels.len(), bufs);
            let (sent, back) = r.levels[1];
            assert_eq!(sent, back);
            assert!((5600..=5700).contains(&back.rms()) && (990..=1010).contains(&back.crossing_hz(16000)));
            assert_eq!((r.levels[0].0.rms(), r.levels[0].1.rms()), (0, 0));
            let jobs = std::mem::take(&mut vpm.transport().jobs);
            assert_eq!(jobs.len(), 20);
            assert!(jobs.iter().all(|j| (j.client, j.opcode, j.body.len()) == (4, 2, if bufs == 3 { 1120 } else { 480 })));
            let text = r.to_string();
            assert!(text.contains("buffer 1 only") && text.contains("returned rms"), "{text}");
            // The speakerphone canceller has two buffers on both families.
            // It returns whole frames once the start-up sequence has set
            // its flag registers; before that the job comes back short.
            let e = canceller_experiment(&mut vpm, Canceller::Sbaec, false, 0, 500, 5).unwrap_err();
            assert!(e.contains("short DSP result"), "{e}");
            startup::run_startup(&mut vpm, &StartupOptions::default(), |_, _| {}).unwrap();
            let r = canceller_experiment(&mut vpm, Canceller::Sbaec, false, 0, 500, 5).unwrap();
            assert_eq!(r.levels.len(), 2);
            assert!(canceller_experiment(&mut vpm, Canceller::Sbaec, false, 2, 500, 5).is_err());
            assert!(canceller_experiment(&mut vpm, Canceller::Hec(0), true, 0, 4000, 5).is_err());
        }
    }

    #[test]
    fn band_experiment_runs() {
        let mut vpm = Vpm::new(MockTransport::new(Family::New)).unwrap();
        let r = band_experiment(&mut vpm, 2, Signal::Sweep, 10).unwrap();
        // Split (operation 1) and combine (operation 4) on stream 2's
        // filter bank, once per frame.
        let jobs = &vpm.transport().jobs;
        assert_eq!(jobs.len(), 20);
        assert_eq!((jobs[0].client, jobs[0].channel, jobs[0].opcode, jobs[0].body.len()), (8, 10, 1, 640));
        assert_eq!((jobs[1].client, jobs[1].channel, jobs[1].opcode, jobs[1].body.len()), (8, 11, 4, 1120));
        assert_eq!(r.combined.len(), 3);
        assert!(r.to_string().contains("combine result, buffer 2"));
        // The old family's DSP has no filter bank.
        let mut vpm = Vpm::new(MockTransport::new(Family::Old)).unwrap();
        assert_eq!(band_experiment(&mut vpm, 2, Signal::Tone(12000), 10), Err(Error::NotOnThisFamily));
    }
}
