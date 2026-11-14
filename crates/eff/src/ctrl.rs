//! The 304-byte bank-selection record stored (replicated) in the `ctrl`
//! partition and read by boot2.
//!
//! ```text
//! 0x000 active side (u32)
//! 0x004 side 0 application header (148 bytes)
//! 0x098 side 1 application header (148 bytes)
//! 0x12c CRC of the preceding 300 bytes
//! ```
//!
//! Integers are big-endian in flash; the CRCs are computed over the record
//! with its ten integers in little-endian form.

use crate::crc::Crc;
use crate::ota::{APP_HEADER_LEN, AppHeader};
use crate::{Result, bad};

pub const CTRL_RECORD_LEN: usize = 304;

/// One bank's header as found in `ctrl`: the raw bytes are kept so that an
/// invalidated header (filled with `0xfe` by the updater) survives a
/// read-modify-write unchanged.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Side {
    pub raw: [u8; APP_HEADER_LEN],
}

impl std::fmt::Debug for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.header() {
            Some(h) => write!(f, "Side({h:?})"),
            None => f.write_str("Side(invalid)"),
        }
    }
}

impl Side {
    /// What the updater writes before reprogramming a bank.
    pub const INVALID: Side = Side {
        raw: [0xfe; APP_HEADER_LEN],
    };

    pub fn from_header(h: &AppHeader) -> Self {
        Side { raw: h.to_bytes() }
    }

    /// The header, if its CRC is valid (boot2's test for a bootable side).
    pub fn header(&self) -> Option<AppHeader> {
        AppHeader::from_bytes(&self.raw).ok()
    }

    pub fn is_valid(&self) -> bool {
        self.header().is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CtrlRecord {
    /// Side boot2 boots when its header is valid: 0 or 1.
    pub active: u32,
    pub sides: [Side; 2],
}

impl CtrlRecord {
    fn crc_of(active: u32, sides: &[Side; 2]) -> u32 {
        let mut c = Crc::new();
        c.update(&active.to_le_bytes());
        for s in sides {
            // Integers at +0, +4, +8, +12, and +144 of each header are
            // byte-swapped before the checksum; the text is not.
            for i in [0usize, 4, 8, 12] {
                let w: [u8; 4] = s.raw[i..i + 4].try_into().unwrap();
                c.update(&u32::from_be_bytes(w).to_le_bytes());
            }
            c.update(&s.raw[16..144]);
            let w: [u8; 4] = s.raw[144..148].try_into().unwrap();
            c.update(&u32::from_be_bytes(w).to_le_bytes());
        }
        c.value()
    }

    /// Parse a record and check its CRC.
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < CTRL_RECORD_LEN {
            return bad("truncated ctrl record");
        }
        let active = u32::from_be_bytes(b[0..4].try_into().unwrap());
        let sides = [
            Side {
                raw: b[4..152].try_into().unwrap(),
            },
            Side {
                raw: b[152..300].try_into().unwrap(),
            },
        ];
        let stored = u32::from_be_bytes(b[300..304].try_into().unwrap());
        if stored != Self::crc_of(active, &sides) {
            return bad("invalid ctrl record CRC");
        }
        Ok(CtrlRecord { active, sides })
    }

    pub fn to_bytes(&self) -> [u8; CTRL_RECORD_LEN] {
        let mut out = [0u8; CTRL_RECORD_LEN];
        out[0..4].copy_from_slice(&self.active.to_be_bytes());
        out[4..152].copy_from_slice(&self.sides[0].raw);
        out[152..300].copy_from_slice(&self.sides[1].raw);
        out[300..304].copy_from_slice(&Self::crc_of(self.active, &self.sides).to_be_bytes());
        out
    }

    /// The side boot2 boots for this record (boot2 main, 0x4404..0x44b0).
    pub fn boot_side(&self) -> u32 {
        let (h0, h1) = (self.sides[0].is_valid(), self.sides[1].is_valid());
        if h0 && self.active == 0 {
            0
        } else if h1 && (self.active == 1 || !h0) {
            1
        } else {
            0
        }
    }
}
