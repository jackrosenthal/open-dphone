// SPDX-License-Identifier: GPL-2.0-or-later
//! One-time DSP jobs observed on a stock D60 (new family) and D40 (old
//! family), compiled in from `data/*.json` by `build.rs`.

use crate::transport::{Reply, Transport};
use crate::vpm::{Call, Vpm, words_to_bytes};
use crate::{Family, Result, abi};

#[derive(Debug)]
pub struct ReplayJob {
    pub t_us: u32,
    pub client: u16,
    pub channel: u16,
    pub instance: u16,
    pub codec: u16,
    pub op: u16,
    pub arg1: u16,
    pub arg2: u16,
    /// Frame length the trace recorded: header words plus body words.
    pub frame_words: u16,
    pub body: &'static [u16],
}

#[derive(Debug)]
pub struct Sequence {
    pub name: &'static str,
    /// Derived from the vendor's modules, not captured.
    pub derived_static: bool,
    pub jobs: &'static [ReplayJob],
}

#[derive(Debug)]
pub struct ReplayTable {
    pub family: Family,
    pub sequences: &'static [Sequence],
}

include!(concat!(env!("OUT_DIR"), "/replay_tables.rs"));

pub fn table(family: Family) -> &'static ReplayTable {
    match family {
        Family::New => &NEW_FAMILY,
        Family::Old => &OLD_FAMILY,
    }
}

impl ReplayTable {
    pub fn sequence(&self, name: &str) -> Option<&'static Sequence> {
        self.sequences.iter().find(|s| s.name == name)
    }
}

/// Header words of a client's job frame.
pub fn header_words(client: u16) -> Option<u16> {
    match client {
        abi::CLIENT_PVE | abi::CLIENT_SBAEC => Some(10),
        abi::CLIENT_HEC | abi::CLIENT_BAND => Some(9),
        _ => None,
    }
}

pub fn client_name(client: u16) -> &'static str {
    match client {
        abi::CLIENT_PVE => "PVE",
        abi::CLIENT_SBAEC => "SBAEC",
        abi::CLIENT_HEC => "HEC",
        abi::CLIENT_BAND => "BAND",
        _ => "?",
    }
}

/// A job that can be sent: a table entry or one read from text.
pub trait JobLike {
    fn client(&self) -> u16;
    fn channel(&self) -> u16;
    fn instance(&self) -> u16;
    fn codec(&self) -> u16;
    fn op(&self) -> u16;
    fn arg1(&self) -> u16;
    fn arg2(&self) -> u16;
    fn body(&self) -> &[u16];

    fn is_query(&self) -> bool {
        self.client() == abi::CLIENT_PVE && (self.op() == 6 || self.op() == 7)
    }

    fn describe(&self) -> String {
        format!(
            "{:<5} chan {:2} inst {} codec {:2} op {:2} args {:04x} {:04x} body {:4} words",
            client_name(self.client()),
            self.channel(),
            self.instance(),
            self.codec(),
            self.op(),
            self.arg1(),
            self.arg2(),
            self.body().len()
        )
    }
}

macro_rules! joblike {
    ($t:ty) => {
        impl JobLike for $t {
            fn client(&self) -> u16 {
                self.client
            }
            fn channel(&self) -> u16 {
                self.channel
            }
            fn instance(&self) -> u16 {
                self.instance
            }
            fn codec(&self) -> u16 {
                self.codec
            }
            fn op(&self) -> u16 {
                self.op
            }
            fn arg1(&self) -> u16 {
                self.arg1
            }
            fn arg2(&self) -> u16 {
                self.arg2
            }
            fn body(&self) -> &[u16] {
                &self.body
            }
        }
    };
}
joblike!(ReplayJob);
joblike!(OwnedJob);

/// Sends one job as recorded.  `reply` receives the result body.
pub fn send<T: Transport>(vpm: &mut Vpm<T>, job: &dyn JobLike, reply: &mut [u8]) -> Result<Reply> {
    let body = words_to_bytes(job.body());
    vpm.job(
        &Call {
            channel: job.channel(),
            client: job.client(),
            instance: job.instance(),
            opcode: job.op(),
            codec: job.codec(),
            arg1: job.arg1(),
            arg2: job.arg2(),
            body: &body,
            timeout_ms: 0,
        },
        reply,
    )
}

/// A job to send, with the name of the sequence it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedJob {
    pub seq: String,
    pub t_us: u64,
    pub client: u16,
    pub channel: u16,
    pub instance: u16,
    pub codec: u16,
    pub op: u16,
    pub arg1: u16,
    pub arg2: u16,
    pub frame_words: u32,
    pub body: Vec<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_consistent() {
        for t in [&NEW_FAMILY, &OLD_FAMILY] {
            assert!(!t.sequences.is_empty());
            for s in t.sequences {
                for j in s.jobs {
                    let h = header_words(j.client).expect("known client");
                    assert_eq!(usize::from(j.frame_words), usize::from(h) + j.body.len(), "{} {}", s.name, j.describe());
                }
            }
        }
    }

    #[test]
    fn known_bodies() {
        let boot = NEW_FAMILY.sequence("boot_part1").unwrap();
        let hec_init = boot.jobs.iter().find(|j| j.client == abi::CLIENT_HEC && j.op == 0).unwrap();
        assert_eq!(
            hec_init.body,
            [0, 0, 0, 0, 0x0600, 0, 0xf100, 0, 0x2000, 0, 0x0600, 0xf100, 0, 0x2000, 0x0600, 0x4800, 0x00a0]
        );
        let sbaec_init = boot.jobs.iter().find(|j| j.client == abi::CLIENT_SBAEC && j.op == 0).unwrap();
        assert_eq!((sbaec_init.arg1, sbaec_init.body.len(), sbaec_init.body[0]), (2, 932, 0x04d3));
        // queries were cleaned of reply values
        assert!(boot.jobs.iter().filter(|j| j.is_query()).all(|j| j.arg1 == 0 && j.arg2 == 0));
        let old = OLD_FAMILY.sequence("boot_module_load_static").unwrap();
        assert!(old.derived_static);
        assert_eq!(old.jobs.len(), 3);
        assert_eq!((old.jobs[2].arg1, old.jobs[2].body.len(), old.jobs[2].body[0]), (2, 931, 0x04d3));
    }
}
