// SPDX-License-Identifier: GPL-2.0-or-later
//! Userspace side of the BCMRING voice DSP ("VPM") job interface.
//!
//! The DSP is a compute coprocessor: the host hands it one job at a time (a
//! codec frame, an echo-canceller frame, a band split, a configuration
//! block) and gets the result back.  One ioctl on `/dev/bcmring-vpm` carries
//! one job.  This crate is a Rust port of the earlier C library
//! (`libbcmvpm`: `vpm.c`).
//!
//! What has run on hardware (through the C library this is a port of): the
//! vocoder query, init, encode, and decode jobs on both families, and the
//! replay of the new family's start-up sequence.  The per-frame canceller
//! and filter-bank wrappers encode what the traces show each job looks like;
//! comments say where a value is inferred.

pub mod abi;
pub mod channels;
pub mod codec;
pub mod diag;
mod error;
pub mod remoteproc;
pub mod replay;
pub mod startup;
pub mod transport;
pub mod vpm;

pub use abi::{Info, Job};
pub use channels::ChannelMap;
pub use codec::{Codec, CodecParams};
pub use error::{Error, Result};
pub use transport::{Device, MockTransport, Transport};
pub use vpm::*;

/// Firmware and SoC family.  They differ in ring size, channels, clients,
/// codec numbering, and the highest sample rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// BCM11109, BCM11170: D40, D45, D50, D70.  16 kHz audio, no BAND client.
    Old,
    /// BCM11188, BCM11189: D60, D62, D65.  32 kHz audio.
    New,
}

impl Family {
    pub fn from_abi(v: u32) -> Option<Self> {
        match v {
            abi::FAMILY_OLD => Some(Family::Old),
            abi::FAMILY_NEW => Some(Family::New),
            _ => None,
        }
    }

    pub fn to_abi(self) -> u32 {
        match self {
            Family::Old => abi::FAMILY_OLD,
            Family::New => abi::FAMILY_NEW,
        }
    }

    /// Sample rate of the audio front end.
    pub fn device_rate(self) -> u32 {
        match self {
            Family::Old => 16_000,
            Family::New => 32_000,
        }
    }

    /// Family from a model name such as `D60`.
    pub fn from_model(model: &str) -> Option<Self> {
        match model.trim().to_ascii_uppercase().as_str() {
            "D40" | "D45" | "D50" | "D70" => Some(Family::Old),
            "D60" | "D62" | "D65" => Some(Family::New),
            _ => None,
        }
    }
}
