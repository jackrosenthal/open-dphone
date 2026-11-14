//! The `ctrl` partition: where the bank-selection record lives and how it
//! is replicated.
//!
//! The 304-byte record itself is [`eff::ctrl::CtrlRecord`].  In flash it is
//! padded to one NAND page with `0xff`.
//!
//! What reads it (established from the boot2 disassembly of both families
//! and from stock `libota`):
//!
//! - boot2 walks the partition one erase block at a time, skips blocks
//!   marked bad, reads the **first page** of each good block, and takes the
//!   first copy whose record CRC is right.  It never looks at other pages.
//! - Stock `ota_GetCtrlBlock` does the same walk.
//!
//! What writes it differs by family; both layouts satisfy both readers:
//!
//! - Old family (D40, D45, D50, D70): one copy at the start of every good
//!   block (four copies seen on a D40 and a D50).
//! - New family (D60, D62, D65): a copy in **every page** of every good
//!   block (256 identical pages seen in a D60 dump and on a live D60 after
//!   a stock update).
//!
//! Stock erases the whole partition and then rewrites it, which leaves a
//! window with no record at all.  [`write`] produces the same final layout
//! but goes one block at a time (erase, write, read back), so that at every
//! instant some good block still starts with a whole record, old or new.
//! Callers arrange that both the old and the new record are safe to boot.

use dphone_hal::MtdPartition;
use dphone_state::Family;
use eff::ctrl::{CTRL_RECORD_LEN, CtrlRecord, Side};

use crate::error::{Result, UpdateError};

/// Name of the partition.
pub const CTRL_PART: &str = "ctrl";

/// How copies of the record are laid out within `ctrl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replication {
    /// One copy in the first page of each good block (old family).
    BlockStart,
    /// A copy in every page of each good block (new family).
    EveryPage,
}

impl Replication {
    pub const fn for_family(family: Family) -> Self {
        match family {
            Family::Old => Replication::BlockStart,
            Family::New => Replication::EveryPage,
        }
    }
}

/// What a walk over `ctrl` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtrlScan {
    /// The record boot2 would use: the first CRC-valid first-page copy.
    pub record: Option<CtrlRecord>,
    /// Index of the block that copy came from.
    pub record_block: Option<u32>,
    pub good_blocks: u32,
    pub bad_blocks: u32,
    /// Good blocks whose first page holds a CRC-valid record.
    pub valid_copies: u32,
    /// All CRC-valid first-page copies are identical.
    pub consistent: bool,
    /// What boot2 is left with when no copy is valid: it keeps going with
    /// the last page it read, unchecked.
    pub fallback: Option<CtrlRecord>,
}

impl CtrlScan {
    /// The record boot2 acts on, checked or not.
    pub fn effective(&self) -> Option<&CtrlRecord> {
        self.record.as_ref().or(self.fallback.as_ref())
    }
}

/// A record's fields without the CRC check (what boot2 has in its buffer
/// when it prints "Control Block is invalid" and carries on).
fn unchecked(b: &[u8]) -> CtrlRecord {
    CtrlRecord {
        active: u32::from_be_bytes(b[0..4].try_into().unwrap()),
        sides: [Side { raw: b[4..152].try_into().unwrap() }, Side { raw: b[152..300].try_into().unwrap() }],
    }
}

/// Walks `ctrl` the way boot2 does.
pub fn scan(part: &mut dyn MtdPartition) -> Result<CtrlScan> {
    let info = part.info().clone();
    let bs = info.erase_size as u64;
    let mut out = CtrlScan {
        record: None,
        record_block: None,
        good_blocks: 0,
        bad_blocks: 0,
        valid_copies: 0,
        consistent: true,
        fallback: None,
    };
    let mut page = [0u8; CTRL_RECORD_LEN];
    let mut block = 0u32;
    let mut off = 0u64;
    while off + bs <= info.size {
        if part.is_bad(off)? {
            out.bad_blocks += 1;
        } else {
            out.good_blocks += 1;
            part.read(off, &mut page)?;
            match CtrlRecord::parse(&page) {
                Ok(rec) => {
                    out.valid_copies += 1;
                    match &out.record {
                        None => {
                            out.record = Some(rec);
                            out.record_block = Some(block);
                        }
                        Some(first) => out.consistent &= *first == rec,
                    }
                }
                Err(_) => out.fallback = Some(unchecked(&page)),
            }
        }
        off += bs;
        block += 1;
    }
    if out.record.is_some() {
        out.fallback = None;
    }
    Ok(out)
}

/// The record boot2 would use; an error when no copy is valid.
pub fn read(part: &mut dyn MtdPartition) -> Result<CtrlRecord> {
    scan(part)?.record.ok_or_else(|| UpdateError::Ctrl("no valid copy in the ctrl partition".into()))
}

/// The image of one erase block holding `record` in the given layout.
fn block_image(record: &CtrlRecord, repl: Replication, page: usize, block: usize) -> Vec<u8> {
    let mut img = vec![0xffu8; block];
    let bytes = record.to_bytes();
    let copies = match repl {
        Replication::BlockStart => 1,
        Replication::EveryPage => block / page,
    };
    for i in 0..copies {
        img[i * page..i * page + CTRL_RECORD_LEN].copy_from_slice(&bytes);
    }
    img
}

/// Writes `record` to every good block of `ctrl`, one block at a time, and
/// confirms that boot2's walk now yields exactly this record.
///
/// Blocks that already hold the wanted contents are left alone, so
/// rewriting the same record costs no erase cycles.  A block that fails to
/// erase, write, or read back is reported in the log and tolerated only if
/// its first page does not hold some *other* valid record; the final check
/// requires every valid first-page copy to be the new record.
pub fn write(part: &mut dyn MtdPartition, record: &CtrlRecord, repl: Replication) -> Result<()> {
    let info = part.info().clone();
    let (bs, page) = (info.erase_size as usize, info.write_size as usize);
    if page < CTRL_RECORD_LEN || bs % page != 0 || bs == 0 {
        return Err(UpdateError::Layout(format!(
            "ctrl geometry (page {page}, block {bs}) cannot hold the record"
        )));
    }
    if record.active > 1 {
        return Err(UpdateError::Refused(format!("active side {} is not 0 or 1", record.active)));
    }
    let want = block_image(record, repl, page, bs);
    // Bytes that must be programmed: up to the end of the last copy's page.
    let used = match repl {
        Replication::BlockStart => page,
        Replication::EveryPage => bs,
    };
    let mut have = vec![0u8; bs];
    let mut written = 0u32;
    let mut failed = 0u32;
    let mut off = 0u64;
    while off + bs as u64 <= info.size {
        if !part.is_bad(off)? {
            let block_result = (|| -> Result<bool> {
                part.read(off, &mut have)?;
                if have == want {
                    return Ok(false);
                }
                part.erase(off)?;
                part.write(off, &want[..used])?;
                part.read(off, &mut have)?;
                if have != want {
                    return Err(UpdateError::Verify(format!("ctrl block at {off:#x} reads back wrong")));
                }
                Ok(true)
            })();
            match block_result {
                Ok(_) => written += 1,
                Err(e) => {
                    failed += 1;
                    log::warn!("ctrl block at {off:#x}: {e}");
                }
            }
        }
        off += bs as u64;
    }
    if written == 0 {
        return Err(UpdateError::Ctrl("no ctrl block could be written".into()));
    }
    // What will boot2 see?
    let after = scan(part)?;
    if after.record.as_ref() != Some(record) || !after.consistent {
        return Err(UpdateError::Ctrl(format!(
            "after writing, boot2 would not read the new record ({} of {} good blocks failed)",
            failed, after.good_blocks
        )));
    }
    if failed > 0 {
        log::warn!("ctrl: {failed} block(s) failed but hold no stale record; {written} good copies");
    }
    Ok(())
}

/// Which side boot2 boots given what a scan found (boot2 `main`,
/// 0x4404..0x44b0 in both families).  With no valid copy boot2 applies the
/// same rule to the unchecked buffer.
pub fn boot_side(scan: &CtrlScan) -> u32 {
    scan.effective().map_or(0, CtrlRecord::boot_side)
}
