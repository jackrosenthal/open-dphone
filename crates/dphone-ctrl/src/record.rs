//! The 304-byte bank-selection record, as raw bytes (the typed form is
//! `eff::ctrl::CtrlRecord`; the updater's tests hold the two together).
//!
//! ```text
//! 0x000 active side (u32)
//! 0x004 side 0 application header (148 bytes, its own CRC last)
//! 0x098 side 1 application header (148 bytes, its own CRC last)
//! 0x12c CRC of the preceding 300 bytes
//! ```
//!
//! Integers are big-endian in flash; the CRCs (reflected CRC-32, initial
//! value `0xffffffff`, no final inversion) are computed with the integers
//! in little-endian form.

pub const RECORD_LEN: usize = 304;
pub const SIDE_LEN: usize = 148;

pub type Record = [u8; RECORD_LEN];

fn crc_update(mut c: u32, data: &[u8]) -> u32 {
    for &b in data {
        c ^= b as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
    }
    c
}

/// CRC step over a big-endian word in its little-endian form.
fn crc_word(c: u32, be: &[u8]) -> u32 {
    crc_update(c, &[be[3], be[2], be[1], be[0]])
}

fn word(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn side(record: &[u8], i: usize) -> &[u8] {
    &record[4 + i * SIDE_LEN..4 + (i + 1) * SIDE_LEN]
}

/// Whether a side's header CRC is right: boot2's test for a bootable side.
pub fn side_valid(record: &[u8], i: usize) -> bool {
    let s = side(record, i);
    let mut c = 0xffff_ffff;
    for w in 0..4 {
        c = crc_word(c, &s[w * 4..]);
    }
    c = crc_update(c, &s[16..144]);
    c == word(&s[144..])
}

fn record_crc(record: &[u8]) -> u32 {
    let mut c = crc_word(0xffff_ffff, record);
    for i in 0..2 {
        let s = side(record, i);
        for w in 0..4 {
            c = crc_word(c, &s[w * 4..]);
        }
        c = crc_update(c, &s[16..144]);
        c = crc_word(c, &s[144..]);
    }
    c
}

/// Whether `b` starts with a record whose CRC is right.
pub fn valid(b: &[u8]) -> bool {
    b.len() >= RECORD_LEN && word(&b[300..]) == record_crc(b)
}

/// The active-side word.
pub fn active(record: &[u8]) -> u32 {
    word(record)
}

/// The side boot2 boots for this record (boot2 `main`, 0x4404..0x44b0).
pub fn boot_side(record: &[u8]) -> u32 {
    let (h0, h1) = (side_valid(record, 0), side_valid(record, 1));
    if h0 && active(record) == 0 {
        0
    } else if h1 && (active(record) == 1 || !h0) {
        1
    } else {
        0
    }
}

/// The same record with another active side and the CRC that goes with it.
pub fn with_active(record: &Record, active: u32) -> Record {
    let mut out = *record;
    out[0..4].copy_from_slice(&active.to_be_bytes());
    let crc = record_crc(&out);
    out[300..304].copy_from_slice(&crc.to_be_bytes());
    out
}
