// SPDX-License-Identifier: GPL-2.0-or-later
//! One-time DSP jobs observed on a stock D60 (new family) and D40 (old
//! family), compiled in from `data/*.json` by `build.rs`, and a reader and
//! writer for the line format the earlier tools used (`rpl.c`).
//!
//! Line format, one record per line (numbers decimal, body words hex):
//!
//! ```text
//! F <family>                          "new" or "old"
//! S <sequence name>
//! J <t_us> <client> <channel> <instance> <codec> <op> <arg1> <arg2> <frame_words> <nbody> <w0> <w1> ...
//! ```

use std::fmt::Write as _;

use crate::transport::{Reply, Transport};
use crate::vpm::{Call, Vpm, words_to_bytes};
use crate::{Error, Family, Result, abi};

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

/// A job read from replay text.
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

/// Replay text, parsed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplayText {
    pub family: Option<Family>,
    pub jobs: Vec<OwnedJob>,
}

impl ReplayText {
    pub fn parse(text: &str) -> Result<Self> {
        let mut out = ReplayText::default();
        let mut seq = String::new();
        for (i, line) in text.lines().enumerate() {
            let bad = |what| Error::Replay { line: i as u32 + 1, what };
            let line = line.trim_end_matches(['\r', '\n']);
            let mut it = line.split_ascii_whitespace();
            match line.as_bytes().first() {
                None | Some(b'#') => continue,
                Some(b'F') => {
                    it.next();
                    out.family = Some(match it.next() {
                        Some("new") => Family::New,
                        Some("old") => Family::Old,
                        _ => return Err(bad("bad family line")),
                    });
                }
                Some(b'S') => {
                    it.next();
                    seq = it.next().ok_or(bad("bad sequence line"))?.to_string();
                }
                Some(b'J') => {
                    it.next();
                    let mut num = || -> Result<u64> { it.next().and_then(|s| s.parse().ok()).ok_or(bad("bad job line")) };
                    let t_us = num()?;
                    let mut f = [0u64; 9];
                    for v in &mut f {
                        *v = num()?;
                    }
                    let [client, channel, instance, codec, op, arg1, arg2, frame_words, nbody] = f;
                    if nbody > 0x8000 || [client, channel, instance, codec, op, arg1, arg2].iter().any(|&v| v > 0xffff) {
                        return Err(bad("value out of range"));
                    }
                    let body: Vec<u16> = it
                        .map(|w| u16::from_str_radix(w, 16))
                        .collect::<std::result::Result<_, _>>()
                        .map_err(|_| bad("bad body word"))?;
                    if body.len() as u64 != nbody {
                        return Err(bad("body words do not match the announced count"));
                    }
                    out.jobs.push(OwnedJob {
                        seq: seq.clone(),
                        t_us,
                        client: client as u16,
                        channel: channel as u16,
                        instance: instance as u16,
                        codec: codec as u16,
                        op: op as u16,
                        arg1: arg1 as u16,
                        arg2: arg2 as u16,
                        frame_words: frame_words as u32,
                        body,
                    });
                }
                _ => return Err(bad("unknown record")),
            }
        }
        Ok(out)
    }
}

/// The compiled-in table in the text format.
pub fn to_text(table: &ReplayTable) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "F {}", if table.family == Family::New { "new" } else { "old" });
    for seq in table.sequences {
        let _ = writeln!(s, "S {}", seq.name);
        for j in seq.jobs {
            let _ = write!(
                s,
                "J {} {} {} {} {} {} {} {} {} {}",
                j.t_us, j.client, j.channel, j.instance, j.codec, j.op, j.arg1, j.arg2, j.frame_words, j.body.len()
            );
            for w in j.body {
                let _ = write!(s, " {w:04x}");
            }
            s.push('\n');
        }
    }
    s
}

/// `list` is comma separated; an entry ending in `*` is a prefix.
pub fn seq_selected(list: &str, seq: &str) -> bool {
    list.split(',').any(|p| match p.strip_suffix('*') {
        Some(prefix) => seq.starts_with(prefix),
        None => p == seq,
    })
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

    #[test]
    fn text_round_trip() {
        for t in [&NEW_FAMILY, &OLD_FAMILY] {
            let text = to_text(t);
            let parsed = ReplayText::parse(&text).unwrap();
            assert_eq!(parsed.family, Some(t.family));
            let n: usize = t.sequences.iter().map(|s| s.jobs.len()).sum();
            assert_eq!(parsed.jobs.len(), n);
            let first = &t.sequences[0].jobs[0];
            assert_eq!(parsed.jobs[0].describe(), first.describe());
        }
        assert!(ReplayText::parse("X nope\n").is_err());
        assert!(ReplayText::parse("J 0 1 1 0 0 7 0 0 10 1\n").is_err());
        assert!(seq_selected("boot*", "boot_part2"));
        assert!(seq_selected("a,app_stop_start", "app_stop_start"));
        assert!(!seq_selected("boot*", "g722_handset_setup"));
    }
}
