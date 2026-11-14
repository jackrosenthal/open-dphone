// SPDX-License-Identifier: GPL-2.0-or-later
//! The kernel ABI of `/dev/bcmring-vpm` (`<linux/bcmring_vpm.h>`, version 1).

pub const ABI_VERSION: u32 = 1;

pub const CLIENT_PVE: u16 = 1;
pub const CLIENT_SBAEC: u16 = 3;
pub const CLIENT_HEC: u16 = 4;
pub const CLIENT_BAND: u16 = 8;

pub const FAMILY_OLD: u32 = 0;
pub const FAMILY_NEW: u32 = 1;

pub const STATE_OFFLINE: u32 = 0;
pub const STATE_ONLINE: u32 = 1;
pub const STATE_FAILED: u32 = 2;

/// `struct bcmring_vpm_info`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub abi_version: u32,
    pub family: u32,
    pub state: u32,
    pub num_channels: u32,
    pub max_body: u32,
    pub jobs: u32,
    pub timeouts: u32,
    pub asserts: u32,
    pub async_events: u32,
    pub stale_results: u32,
    pub reserved: [u32; 6],
}

/// `struct bcmring_vpm_job`: one synchronous job.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Job {
    pub channel: u16,
    pub client: u16,
    pub instance: u16,
    pub opcode: u16,
    pub codec: u16,
    /// In: first argument.  Out: the result's first value.
    pub arg1: u16,
    pub arg2: u16,
    /// Out: the body-length word of the result header.
    pub reply_len_field: u16,
    pub body_len: u32,
    pub reply_max: u32,
    /// Out: result body bytes as carried; may exceed `reply_max`.
    pub reply_len: u32,
    /// 0 selects the driver default (1000 ms).
    pub timeout_ms: u32,
    pub body: u64,
    pub reply: u64,
}

const IOC_MAGIC: u32 = 0xb9;
const IOC_READ: u32 = 2;
const IOC_WRITE: u32 = 1;

const fn ioc(dir: u32, nr: u32, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | (IOC_MAGIC << 8) | nr
}

/// `_IOR(0xb9, 0, struct bcmring_vpm_info)`
pub const IOC_INFO: u32 = ioc(IOC_READ, 0, size_of::<Info>());
/// `_IOWR(0xb9, 1, struct bcmring_vpm_job)`
pub const IOC_JOB: u32 = ioc(IOC_READ | IOC_WRITE, 1, size_of::<Job>());

const _: () = assert!(size_of::<Info>() == 64);
const _: () = assert!(size_of::<Job>() == 48);
const _: () = assert!(IOC_INFO == 0x8040_b900);
const _: () = assert!(IOC_JOB == 0xc030_b901);
