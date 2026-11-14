// SPDX-License-Identifier: GPL-2.0-or-later
//! How a job reaches the DSP: the kernel device, or a mock for tests.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;

use crate::abi::{self, Info, Job};
use crate::{Error, Family, Result};

pub const DEFAULT_DEVICE: &str = "/dev/bcmring-vpm";

/// One request as the transport sees it.
#[derive(Debug)]
pub struct Request<'a> {
    pub channel: u16,
    pub client: u16,
    pub instance: u16,
    pub opcode: u16,
    pub codec: u16,
    pub arg1: u16,
    pub arg2: u16,
    pub body: &'a [u8],
    /// 0 selects the driver default (1000 ms).
    pub timeout_ms: u32,
}

/// What came back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub value1: u16,
    pub value2: u16,
    /// The body-length word of the result header.  The DSP leaves 0 here
    /// for some results that carry a body.
    pub len_field: u16,
    /// Result body bytes as carried by the transport; may exceed the buffer.
    pub len: usize,
}

impl Reply {
    /// Bytes stored in a buffer of `max` bytes.
    pub fn stored(&self, max: usize) -> usize {
        self.len.min(max)
    }

    /// How many bytes of the result are meaningful: the header's length word
    /// if it is plausible, otherwise what the transport carried.
    pub fn bytes(&self, max: usize) -> usize {
        let n = self.stored(max);
        if self.len_field != 0 && usize::from(self.len_field) <= n { usize::from(self.len_field) } else { n }
    }
}

pub trait Transport {
    fn info(&mut self) -> Result<Info>;
    fn job(&mut self, req: &Request<'_>, reply: &mut [u8]) -> Result<Reply>;
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn info(&mut self) -> Result<Info> {
        (**self).info()
    }
    fn job(&mut self, req: &Request<'_>, reply: &mut [u8]) -> Result<Reply> {
        (**self).job(req, reply)
    }
}

/// `/dev/bcmring-vpm`.
#[derive(Debug)]
pub struct Device {
    file: File,
}

impl Device {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Self { file })
    }

    pub fn open_default() -> Result<Self> {
        Self::open(DEFAULT_DEVICE)
    }
}

impl Transport for Device {
    fn info(&mut self) -> Result<Info> {
        let mut info = Info::default();
        // SAFETY: the ioctl writes one `Info` to the pointer.
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), abi::IOC_INFO as _, &mut info as *mut Info) };
        if r < 0 {
            return Err(Error::last_os());
        }
        Ok(info)
    }

    fn job(&mut self, req: &Request<'_>, reply: &mut [u8]) -> Result<Reply> {
        let mut job = Job {
            channel: req.channel,
            client: req.client,
            instance: req.instance,
            opcode: req.opcode,
            codec: req.codec,
            arg1: req.arg1,
            arg2: req.arg2,
            reply_len_field: 0,
            body_len: req.body.len() as u32,
            reply_max: reply.len() as u32,
            reply_len: 0,
            timeout_ms: req.timeout_ms,
            body: if req.body.is_empty() { 0 } else { req.body.as_ptr() as usize as u64 },
            reply: if reply.is_empty() { 0 } else { reply.as_mut_ptr() as usize as u64 },
        };
        // SAFETY: the pointers in `job` are valid for the lengths given for
        // the duration of the call.
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), abi::IOC_JOB as _, &mut job as *mut Job) };
        if r < 0 {
            return Err(Error::last_os());
        }
        Ok(Reply { value1: job.arg1, value2: job.arg2, len_field: job.reply_len_field, len: job.reply_len as usize })
    }
}

/// A job as recorded by [`MockTransport`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub channel: u16,
    pub client: u16,
    pub instance: u16,
    pub opcode: u16,
    pub codec: u16,
    pub arg1: u16,
    pub arg2: u16,
    pub body: Vec<u8>,
}

type Handler = Box<dyn FnMut(&Request<'_>, &mut [u8]) -> Result<Reply> + Send>;

/// A transport without hardware.  It records every job and answers with
/// results of the right size: frames are returned unchanged, queries and
/// state polls return zeros, and the two captured codecs get a stand-in
/// that keeps frame sizes (not a codec).  A handler can replace the answers.
pub struct MockTransport {
    pub family: Family,
    pub state: u32,
    pub jobs: Vec<Recorded>,
    /// Keep at most this many recorded jobs (oldest dropped); 0 = unlimited.
    pub keep: usize,
    handler: Option<Handler>,
    count: u32,
}

impl MockTransport {
    pub fn new(family: Family) -> Self {
        Self { family, state: abi::STATE_ONLINE, jobs: Vec::new(), keep: 0, handler: None, count: 0 }
    }

    pub fn with_handler(family: Family, handler: impl FnMut(&Request<'_>, &mut [u8]) -> Result<Reply> + Send + 'static) -> Self {
        let mut t = Self::new(family);
        t.handler = Some(Box::new(handler));
        t
    }

    /// The default answers, usable from a custom handler as a fallback.
    pub fn default_answer(family: Family, req: &Request<'_>, reply: &mut [u8]) -> Result<Reply> {
        let new = family == Family::New;
        let mut r = Reply { value1: req.arg1, value2: req.arg2, len_field: 0, len: 0 };
        let mut fill = |data: &[u8]| {
            let n = data.len().min(reply.len());
            reply[..n].copy_from_slice(&data[..n]);
            data.len()
        };
        match (req.client, req.opcode) {
            (abi::CLIENT_PVE, 7) => {
                let count = if new { 76 } else { 60 };
                r.value1 = count;
                r.len = fill(&vec![0u8; 4 * usize::from(count)]);
            }
            (abi::CLIENT_PVE, 6) => r.len = fill(&[0u8; 536]),
            (abi::CLIENT_PVE, 2) => {
                // Stand-in "encoder": keeps sizes (4:1), not a codec.
                let out: Vec<u8> = req.body.as_chunks::<4>().0.iter().map(|c| c[1]).collect();
                r.value1 = 1;
                r.value2 = 0;
                r.len_field = out.len() as u16;
                r.len = fill(&out);
            }
            (abi::CLIENT_PVE, 5) => {
                let valid = if new { usize::from(req.arg2) } else if req.codec == 4 { 20 } else { 80 };
                let valid = valid.min(req.body.len());
                let mut out = Vec::with_capacity(valid * 4);
                for &b in &req.body[..valid] {
                    out.extend_from_slice(&[0, b, 0, b]);
                }
                r.len_field = out.len() as u16;
                r.len = fill(&out);
            }
            (abi::CLIENT_HEC, 2 | 3) | (abi::CLIENT_SBAEC, 8 | 9) => r.len = fill(req.body),
            (abi::CLIENT_HEC, 8) => r.len = fill(&[0u8; 48]),
            (abi::CLIENT_SBAEC, 7) => r.len = fill(&[0u8; 40]),
            (abi::CLIENT_HEC, 14) => r.len = fill(&[0u8; 6]),
            (abi::CLIENT_BAND, _) if !new => return Err(Error::Invalid),
            (abi::CLIENT_BAND, 1) => {
                // 8n in, 14n out: crude decimation, enough to keep sizes.
                let n = usize::from(req.arg1);
                let mut out = vec![0u8; 14 * n];
                for i in 0..n.min(req.body.len() / 8) {
                    out[2 * i..2 * i + 2].copy_from_slice(&req.body[8 * i..8 * i + 2]);
                }
                r.len = fill(&out);
            }
            (abi::CLIENT_BAND, 2) => {
                let n = usize::from(req.arg1);
                let mut out = vec![0u8; 6 * n];
                for i in 0..n.min(req.body.len() / 4) {
                    out[2 * i..2 * i + 2].copy_from_slice(&req.body[4 * i..4 * i + 2]);
                }
                r.len = fill(&out);
            }
            (abi::CLIENT_BAND, 4) => {
                let n = usize::from(req.arg1);
                let mut out = req.body.to_vec();
                out.resize(14 * n, 0);
                for i in 0..n {
                    let s = [out[2 * i], out[2 * i + 1]];
                    for k in 0..2 {
                        out[2 * n + 4 * i + 2 * k..][..2].copy_from_slice(&s);
                    }
                    for k in 0..4 {
                        out[6 * n + 8 * i + 2 * k..][..2].copy_from_slice(&s);
                    }
                }
                r.len = fill(&out);
            }
            (abi::CLIENT_BAND, 5) => {
                let n = usize::from(req.arg1);
                let mut out = req.body.to_vec();
                out.resize(6 * n, 0);
                for i in 0..n {
                    let s = [out[2 * i], out[2 * i + 1]];
                    for k in 0..2 {
                        out[2 * n + 4 * i + 2 * k..][..2].copy_from_slice(&s);
                    }
                }
                r.len = fill(&out);
            }
            _ => {}
        }
        Ok(r)
    }
}

impl Transport for MockTransport {
    fn info(&mut self) -> Result<Info> {
        Ok(Info {
            abi_version: abi::ABI_VERSION,
            family: self.family.to_abi(),
            state: self.state,
            num_channels: if self.family == Family::New { 32 } else { 16 },
            max_body: 4096,
            jobs: self.count,
            ..Info::default()
        })
    }

    fn job(&mut self, req: &Request<'_>, reply: &mut [u8]) -> Result<Reply> {
        if self.state != abi::STATE_ONLINE {
            return Err(if self.state == abi::STATE_FAILED { Error::Failed } else { Error::NotStarted });
        }
        self.count = self.count.wrapping_add(1);
        if self.keep != 0 && self.jobs.len() >= self.keep {
            self.jobs.remove(0);
        }
        self.jobs.push(Recorded {
            channel: req.channel,
            client: req.client,
            instance: req.instance,
            opcode: req.opcode,
            codec: req.codec,
            arg1: req.arg1,
            arg2: req.arg2,
            body: req.body.to_vec(),
        });
        match &mut self.handler {
            Some(h) => h(req, reply),
            None => Self::default_answer(self.family, req, reply),
        }
    }
}
