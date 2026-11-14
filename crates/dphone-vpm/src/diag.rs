// SPDX-License-Identifier: GPL-2.0-or-later
//! Diagnostics for the `dphone` debugging subcommands: a hardware-free
//! self-test, the G.722 loop, codec queries, replay, and single-job probes
//! of the cancellers and the filter bank.

use crate::codec::{Codec, swap_pairs};
use crate::replay::{self, JobLike, OwnedJob};
use crate::startup::{self, StartupOptions};
use crate::transport::{MockTransport, Reply, Transport};
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
        r.check((j.client, j.channel, j.opcode, j.arg1, j.body.len()) == (3, if new { 18 } else { 8 }, 8, 240, 480), || {
            format!("{fam}: SBAEC egress request {j:?}")
        });
        let j = &jobs[6];
        r.check((j.opcode, j.arg1, j.body.len()) == (9, 240, 480), || format!("{fam}: SBAEC ingress request {j:?}"));
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

/// Sends replay jobs: the compiled-in sequences selected by `seqs` (see
/// [`replay::seq_selected`]), or jobs parsed from text.
pub fn replay_jobs<T: Transport>(
    vpm: &mut Vpm<T>,
    jobs: &[OwnedJob],
    skip_queries: bool,
    keep_going: bool,
    mut observe: impl FnMut(&OwnedJob, &Result<Reply>, &[u8]),
) -> Result<(u32, u32)> {
    let (mut sent, mut failed) = (0, 0);
    let mut reply = vec![0u8; 16384];
    for job in jobs {
        if skip_queries && job.is_query() {
            continue;
        }
        let r = replay::send(vpm, job, &mut reply);
        let n = r.as_ref().map(|r| r.stored(reply.len())).unwrap_or(0);
        observe(job, &r, &reply[..n]);
        match r {
            Ok(_) => sent += 1,
            Err(e) => {
                failed += 1;
                if !keep_going || e.is_fatal() {
                    return Err(e);
                }
            }
        }
    }
    Ok((sent, failed))
}

/// The compiled-in jobs of the sequences selected by `seqs`.
pub fn table_jobs(family: Family, seqs: &str) -> Vec<OwnedJob> {
    replay::ReplayText::parse(&replay::to_text(replay::table(family)))
        .map(|t| t.jobs.into_iter().filter(|j| replay::seq_selected(seqs, &j.seq)).collect())
        .unwrap_or_default()
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
        assert!(r.checks > 300);
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
    fn table_selection() {
        let jobs = table_jobs(Family::New, "g722_handset_setup");
        assert_eq!(jobs.len(), 5);
        let mut vpm = Vpm::new(MockTransport::new(Family::New)).unwrap();
        assert_eq!(replay_jobs(&mut vpm, &jobs, false, false, |_, _, _| {}).unwrap(), (5, 0));
    }
}
