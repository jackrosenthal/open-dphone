// SPDX-License-Identifier: GPL-2.0-or-later
//! What to send once after the DSP has been started, and the tuning data to
//! re-apply later.
//!
//! New family: the stock firmware's complete start-up sequence as captured
//! on a D60 (`boot_part1` to `boot_part3`); this exact replay has been
//! accepted by a D60's DSP under the mainline driver.
//!
//! Old family: the head of the sequence was never captured.  The three Init
//! jobs and the codec-query sweep are known from the vendor's modules (the
//! same reading reproduces the new family's capture word for word); the
//! register loads that follow were captured on a D40.  The order of that
//! tail mirrors the new family's and is an inference; so is the headset
//! canceller's register file (the handset's with the headset's gain).

use crate::replay::{self, JobLike, OwnedJob, ReplayJob};
use crate::transport::{Reply, Transport};
use crate::vpm::{Vpm, hec, sbaec};
use crate::{ChannelMap, Error, Family, Result, abi};

/// Codecs the old family's host routes to the DSP, in the order it queries
/// them: 2 to 7, 9 to 11, 14, 15, 18 to 33, 36, 37, and 40 to 68.
pub fn old_family_query_codecs() -> impl Iterator<Item = u16> {
    (2..=7).chain(9..=11).chain([14, 15]).chain(18..=33).chain([36, 37]).chain(40..=68)
}

#[derive(Clone, Copy, Debug)]
pub struct StartupOptions {
    /// Send the codec queries (capability discovery; the firmware may not
    /// need them, but the stock firmware always sends them).
    pub queries: bool,
    /// Keep going after a failed job, unless the failure is fatal.
    pub keep_going: bool,
}

impl Default for StartupOptions {
    fn default() -> Self {
        Self { queries: true, keep_going: false }
    }
}

fn owned(seq: &str, j: &ReplayJob) -> OwnedJob {
    OwnedJob {
        seq: seq.to_string(),
        t_us: u64::from(j.t_us),
        client: j.client,
        channel: j.channel,
        instance: j.instance,
        codec: j.codec,
        op: j.op,
        arg1: j.arg1,
        arg2: j.arg2,
        frame_words: u32::from(j.frame_words),
        body: j.body.to_vec(),
    }
}

#[allow(clippy::too_many_arguments)]
fn synth(seq: &str, client: u16, channel: u16, instance: u16, codec: u16, op: u16, arg1: u16, arg2: u16, body: &[u16]) -> OwnedJob {
    OwnedJob {
        seq: seq.to_string(),
        t_us: 0,
        client,
        channel,
        instance,
        codec,
        op,
        arg1,
        arg2,
        frame_words: u32::from(replay::header_words(client).unwrap_or(9)) + body.len() as u32,
        body: body.to_vec(),
    }
}

/// Default receive gains in dB * 256 as the stock firmware sets them:
/// handset -9, headset +2, speakerphone -3 (new family) or +1 (old).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultGains {
    pub handset: i16,
    pub headset: i16,
    pub speaker: i16,
}

/// Canceller register files for re-application (e.g., after a DSP restart).
#[derive(Clone, Debug)]
pub struct Tuning {
    pub family: Family,
    /// 17 words each: handset, headset.
    pub hec: [[u16; hec::NUM_REGS]; 2],
    /// 932 words (new family) or 931 (old).
    pub sbaec: &'static [u16],
    pub gains: DefaultGains,
}

/// The tuning captured for a family.  It is one D60's and one D40's; other
/// models of the family get it until their own is captured.
pub fn tuning(family: Family) -> Tuning {
    let t = replay::table(family);
    let find = |seq: &str, client: u16, op: u16, channel: Option<u16>| -> &'static [u16] {
        t.sequence(seq)
            .and_then(|s| s.jobs.iter().find(|j| j.client == client && j.op == op && channel.is_none_or(|c| c == j.channel)))
            .map(|j| j.body)
            .expect("tuning job present in the replay table")
    };
    let map = ChannelMap::new(family);
    let (sbaec_regs, gains) = match family {
        Family::New => (
            find("boot_part1", abi::CLIENT_SBAEC, sbaec::SET_REGS, None),
            DefaultGains { handset: -9 * 256, headset: 2 * 256, speaker: -3 * 256 },
        ),
        Family::Old => (
            find("boot_static_images", abi::CLIENT_SBAEC, sbaec::SET_REGS, None),
            DefaultGains { handset: -9 * 256, headset: 2 * 256, speaker: 256 },
        ),
    };
    // The handset's captured register file (word 2 is its receive gain).
    let handset: [u16; hec::NUM_REGS] =
        find("app_stop_start", abi::CLIENT_HEC, hec::SET_REGS, Some(map.hec(0))).try_into().expect("17 words");
    // The headset's: captured on the new family; on the old family the
    // handset's file with the headset's gain (inferred).
    let headset: [u16; hec::NUM_REGS] = match family {
        Family::New => find("boot_part2", abi::CLIENT_HEC, hec::SET_REGS, Some(map.hec(1))).try_into().expect("17 words"),
        Family::Old => {
            let mut r = handset;
            r[2] = gains.headset as u16;
            r
        }
    };
    Tuning { family, hec: [handset, headset], sbaec: sbaec_regs, gains }
}

/// The jobs to send once after the DSP has started, in order.
pub fn startup_jobs(family: Family, opts: &StartupOptions) -> Vec<OwnedJob> {
    let t = replay::table(family);
    let mut out = Vec::new();
    match family {
        Family::New => {
            for name in ["boot_part1", "boot_part2", "boot_part3"] {
                let seq = t.sequence(name).expect("boot sequence present");
                out.extend(
                    seq.jobs
                        .iter()
                        .filter(|j| opts.queries || !j.is_query())
                        // The capture holds the speakerphone canceller's
                        // operation 1 three times, with argument 1, 0, and
                        // 0; the two zeros sit at capture boundaries and
                        // may be results caught as requests.  Measured on a
                        // D60: argument 1 leaves register 0 at 0 and
                        // argument 0 leaves it at 0x04d3, and stock at idle
                        // reads register 0 as 0.  Only the argument-1 form
                        // is sent, so the idle state matches stock's.
                        .filter(|j| !(j.client == abi::CLIENT_SBAEC && j.op == sbaec::ENABLE && j.arg1 == 0))
                        .map(|j| owned(name, j)),
                );
            }
            // The vendor's last step for the speakerphone canceller, after
            // its register load and "set register 0 = 1", is operation 1
            // with argument 1 (register 0 then reads 0, as on idle stock).
            let sb = ChannelMap::new(family).sbaec();
            let at = out.iter().rposition(|j| j.client == abi::CLIENT_SBAEC).map_or(out.len(), |i| i + 1);
            out.insert(at, synth("boot_tail", abi::CLIENT_SBAEC, sb, 0, 0, sbaec::ENABLE, 1, 0, &[]));
        }
        Family::Old => {
            let map = ChannelMap::new(family);
            if opts.queries {
                for stream in 0..crate::channels::NUM_STREAMS {
                    let (_, dec) = map.pve(stream);
                    out.push(synth("query_sweep_static", abi::CLIENT_PVE, dec, stream as u16, 0, 7, 0, 0, &[]));
                    for codec in old_family_query_codecs() {
                        out.push(synth("query_sweep_static", abi::CLIENT_PVE, dec, stream as u16, codec, 6, 0, 0, &[]));
                    }
                }
            }
            let name = "boot_module_load_static";
            out.extend(t.sequence(name).expect("static init sequence").jobs.iter().map(|j| owned(name, j)));
            // The tail, in the new family's order.
            let tune = tuning(family);
            let (h0, h1, sb) = (map.hec(0), map.hec(1), map.sbaec());
            let tail = "boot_tail_inferred";
            out.push(synth(tail, abi::CLIENT_HEC, h0, 0, 0, hec::ENABLE, 1, 0, &[]));
            out.push(synth(tail, abi::CLIENT_SBAEC, sb, 0, 0, sbaec::SET_REG, sbaec::REG_ACTIVE, 1, &[]));
            out.push(synth(tail, abi::CLIENT_SBAEC, sb, 0, 0, sbaec::ENABLE, 1, 0, &[]));
            out.push(synth(tail, abi::CLIENT_SBAEC, sb, 0, 0, sbaec::SET_REGS, 0, tune.sbaec.len() as u16 - 1, tune.sbaec));
            out.push(synth(tail, abi::CLIENT_HEC, h0, 0, 0, hec::SET_REGS, 0, 0, &tune.hec[0]));
            out.push(synth(tail, abi::CLIENT_HEC, h1, 1, 0, hec::SET_REGS, 0, 0, &tune.hec[1]));
            out.push(synth(tail, abi::CLIENT_SBAEC, sb, 0, 0, sbaec::SET_REG, sbaec::REG_ACTIVE, 1, &[]));
            out.push(synth(tail, abi::CLIENT_HEC, h1, 1, 0, hec::ENABLE, 1, 0, &[]));
        }
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupReport {
    pub sent: u32,
    pub failed: u32,
    /// The first failure: job description and error.
    pub first_error: Option<(String, Error)>,
}

/// Sends the start-up sequence.  The DSP must be online.  `observe` sees
/// every job and its outcome (for logs and diagnostics).
pub fn run_startup<T: Transport>(
    vpm: &mut Vpm<T>,
    opts: &StartupOptions,
    mut observe: impl FnMut(&OwnedJob, &Result<Reply>),
) -> Result<StartupReport> {
    vpm.refresh_info()?;
    if !vpm.is_online() {
        return Err(Error::NotStarted);
    }
    let mut report = StartupReport::default();
    let mut reply = vec![0u8; 4096];
    for job in startup_jobs(vpm.family(), opts) {
        let r = replay::send(vpm, &job, &mut reply);
        observe(&job, &r);
        match r {
            Ok(_) => report.sent += 1,
            Err(e) => {
                report.failed += 1;
                let fatal = e.is_fatal();
                if report.first_error.is_none() {
                    report.first_error = Some((job.describe(), e.clone()));
                }
                if fatal || !opts.keep_going {
                    return Err(e);
                }
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockTransport;

    #[test]
    fn new_family_sequence() {
        let jobs = startup_jobs(Family::New, &StartupOptions::default());
        // what a D60 accepted: about 220 jobs
        assert!((215..=230).contains(&jobs.len()), "{}", jobs.len());
        let non_query: Vec<_> = jobs.iter().filter(|j| !j.is_query()).collect();
        // HEC 0 Init comes before its Enable; SBAEC Init before its register load
        let pos = |client, op, chan| non_query.iter().position(|j| j.client == client && j.op == op && j.channel == chan).unwrap();
        assert!(pos(abi::CLIENT_HEC, hec::INIT, 12) < pos(abi::CLIENT_HEC, hec::ENABLE, 12));
        assert!(pos(abi::CLIENT_SBAEC, sbaec::INIT, 18) < pos(abi::CLIENT_SBAEC, sbaec::SET_REGS, 18));
        assert!(pos(abi::CLIENT_HEC, hec::INIT, 15) < pos(abi::CLIENT_HEC, hec::ENABLE, 15));
        // the speakerphone's filter bank is initialized last
        let last = non_query.last().unwrap();
        assert_eq!((last.client, last.channel, last.op), (abi::CLIENT_BAND, 20, 3));
        let without = startup_jobs(Family::New, &StartupOptions { queries: false, ..Default::default() });
        assert_eq!(without.len(), non_query.len());
    }

    #[test]
    fn old_family_sequence() {
        let jobs = startup_jobs(Family::Old, &StartupOptions::default());
        assert_eq!(old_family_query_codecs().count(), 58);
        assert_eq!(jobs.iter().filter(|j| j.is_query()).count(), 3 * 59);
        let rest: Vec<_> = jobs.iter().filter(|j| !j.is_query()).collect();
        assert_eq!(rest.len(), 3 + 8);
        assert_eq!((rest[0].client, rest[0].channel, rest[0].op, rest[0].body.len()), (abi::CLIENT_HEC, 6, 0, 17));
        assert_eq!((rest[2].client, rest[2].channel, rest[2].arg1, rest[2].body.len()), (abi::CLIENT_SBAEC, 8, 2, 931));
        // every frame fits the old family's largest body
        assert!(jobs.iter().all(|j| j.body.len() <= 931));
    }

    #[test]
    fn tuning_tables() {
        let n = tuning(Family::New);
        assert_eq!(n.sbaec.len(), 932);
        assert_eq!(n.hec[0][2], 0xf700);
        let o = tuning(Family::Old);
        assert_eq!(o.sbaec.len(), 931);
        assert_eq!(o.hec[1][2], 0x0200);
    }

    #[test]
    fn runs_against_mock() {
        for family in [Family::New, Family::Old] {
            let mut vpm = Vpm::new(MockTransport::new(family)).unwrap();
            let mut seen = 0;
            let r = run_startup(&mut vpm, &StartupOptions::default(), |_, r| {
                assert!(r.is_ok());
                seen += 1;
            })
            .unwrap();
            assert_eq!(r.failed, 0);
            assert_eq!(r.sent, seen);
            assert_eq!(vpm.transport().jobs.len() as u32, seen);
        }
        let mut t = MockTransport::new(Family::New);
        t.state = abi::STATE_OFFLINE;
        let mut vpm = Vpm::new(t).unwrap();
        assert_eq!(run_startup(&mut vpm, &StartupOptions::default(), |_, _| {}), Err(Error::NotStarted));
    }
}
