// SPDX-License-Identifier: GPL-2.0-or-later
//! The vendor's channel map.  Channel numbers are the caller's choice (the
//! DSP learns of a channel from the first job that names it), but the stock
//! firmware always ends up with this allocation, so it is the safe one.

use crate::Family;

/// Voice streams the vendor opens.
pub const NUM_STREAMS: usize = 3;

/// Channels and instance of one filter bank (new family).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BandChannels {
    pub split: u16,
    pub combine: u16,
    pub instance: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelMap {
    pub family: Family,
}

impl ChannelMap {
    pub fn new(family: Family) -> Self {
        Self { family }
    }

    /// (encode channel, decode channel) of voice stream 0..3.  The PVE
    /// instance of a stream is the stream number.
    pub fn pve(&self, stream: usize) -> (u16, u16) {
        let s = stream as u16;
        match self.family {
            Family::New => (4 * s, 4 * s + 1),
            Family::Old => (2 * s, 2 * s + 1),
        }
    }

    /// Filter bank of voice stream 0..3 (new family).
    pub fn stream_band(&self, stream: usize) -> Option<BandChannels> {
        let s = stream as u16;
        (self.family == Family::New).then_some(BandChannels { split: 4 * s + 2, combine: 4 * s + 3, instance: s })
    }

    /// HEC instance 0 (handset) or 1 (headset; inferred): channel.
    pub fn hec(&self, instance: u16) -> u16 {
        match self.family {
            Family::New => 12 + 3 * instance,
            Family::Old => 6 + instance,
        }
    }

    /// Filter bank beside a HEC instance (new family).
    pub fn hec_band(&self, instance: u16) -> Option<BandChannels> {
        (self.family == Family::New).then_some(BandChannels {
            split: 13 + 3 * instance,
            combine: 14 + 3 * instance,
            instance: 3 + instance,
        })
    }

    /// The speakerphone canceller's channel (instance 0).
    pub fn sbaec(&self) -> u16 {
        match self.family {
            Family::New => 18,
            Family::Old => 8,
        }
    }

    /// Filter bank beside the speakerphone canceller (new family).
    pub fn sbaec_band(&self) -> Option<BandChannels> {
        (self.family == Family::New).then_some(BandChannels { split: 19, combine: 20, instance: 5 })
    }

    /// First channel the vendor leaves unused (what its module hands a new
    /// client; diagnostics that must not disturb anything can use it).
    pub fn first_free(&self) -> u16 {
        match self.family {
            Family::New => 21,
            Family::Old => 9,
        }
    }
}
