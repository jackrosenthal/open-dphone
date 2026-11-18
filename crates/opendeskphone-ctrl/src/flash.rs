//! Reading `ctrl` the way boot2 does, and rewriting it so that boot2 finds
//! a whole record at every instant.
//!
//! What reads the partition: boot2 (both families) walks it one erase block
//! at a time, skips blocks marked bad, reads the **first page** of each good
//! block, and takes the first copy whose record CRC is right.  Stock
//! `libota` does the same walk.
//!
//! What writes it differs by family; both layouts satisfy both readers:
//! one copy at the start of every good block (old family), or a copy in
//! every page of every good block (new family).

use crate::record::{self, RECORD_LEN, Record};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub size: u64,
    pub erase_size: u32,
    pub write_size: u32,
}

/// One NAND partition.  Offsets are in bytes from its start; `erase` takes
/// the start of a block, and `write` starts on a page boundary and pads a
/// last partial page itself.
pub trait Flash {
    type Error;
    fn geometry(&self) -> Geometry;
    fn is_bad(&mut self, offset: u64) -> Result<bool, Self::Error>;
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Self::Error>;
    fn erase(&mut self, offset: u64) -> Result<(), Self::Error>;
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), Self::Error>;
}

/// How copies of the record are laid out within `ctrl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replication {
    /// One copy in the first page of each good block (old family).
    BlockStart,
    /// A copy in every page of each good block (new family).
    EveryPage,
}

/// What a walk over `ctrl` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    /// The record boot2 would use: the first CRC-valid first-page copy.
    pub record: Option<Record>,
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
    pub fallback: Option<Record>,
}

/// Walks `ctrl` the way boot2 does.
pub fn scan<F: Flash + ?Sized>(part: &mut F) -> Result<Scan, F::Error> {
    let g = part.geometry();
    let bs = g.erase_size as u64;
    let mut out = Scan {
        record: None,
        record_block: None,
        good_blocks: 0,
        bad_blocks: 0,
        valid_copies: 0,
        consistent: true,
        fallback: None,
    };
    let mut page = [0u8; RECORD_LEN];
    let mut block = 0u32;
    let mut off = 0u64;
    while bs != 0 && off + bs <= g.size {
        if part.is_bad(off)? {
            out.bad_blocks += 1;
        } else {
            out.good_blocks += 1;
            part.read(off, &mut page)?;
            if record::valid(&page) {
                out.valid_copies += 1;
                match &out.record {
                    None => {
                        out.record = Some(page);
                        out.record_block = Some(block);
                    }
                    Some(first) => out.consistent &= *first == page,
                }
            } else {
                out.fallback = Some(page);
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

/// Why [`write`] did not leave the new record in `ctrl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteError<E> {
    /// A flash operation outside the per-block rewrite failed.
    Flash(E),
    /// The partition's geometry cannot hold the record, or `scratch` is
    /// smaller than two erase blocks.
    Geometry,
    /// The record's active side is not 0 or 1.
    BadActive,
    /// No block could be written.
    NoBlockWritten,
    /// After writing, boot2 would not read the new record.
    NotReadBack { failed: u32, good: u32 },
}

/// Writes `record` to every good block of `ctrl`, one block at a time
/// (erase, write, read back), and confirms that boot2's walk now yields
/// exactly this record.  At every instant some good block still starts
/// with a whole record, old or new; callers arrange that both are safe to
/// boot.  (Stock erases the whole partition first, which leaves a window
/// with no record at all.)
///
/// Blocks that already hold the wanted contents are left alone, so
/// rewriting the same record costs no erase cycles.  A block that fails to
/// erase, write, or read back is tolerated only if its first page does not
/// hold some *other* valid record: the final check requires every valid
/// first-page copy to be the new record.  Returns the number of blocks
/// that failed.
///
/// `scratch` must hold two erase blocks.
pub fn write<F: Flash + ?Sized>(
    part: &mut F,
    record: &Record,
    repl: Replication,
    scratch: &mut [u8],
) -> Result<u32, WriteError<F::Error>> {
    let g = part.geometry();
    let (bs, page) = (g.erase_size as usize, g.write_size as usize);
    if page < RECORD_LEN || bs == 0 || !bs.is_multiple_of(page) || scratch.len() < 2 * bs {
        return Err(WriteError::Geometry);
    }
    if record::active(record) > 1 {
        return Err(WriteError::BadActive);
    }
    let (want, have) = scratch.split_at_mut(bs);
    let have = &mut have[..bs];
    // The image of one erase block, and how much of it must be programmed:
    // up to the end of the last copy's page.
    want.fill(0xff);
    let (copies, used) = match repl {
        Replication::BlockStart => (1, page),
        Replication::EveryPage => (bs / page, bs),
    };
    for i in 0..copies {
        want[i * page..i * page + RECORD_LEN].copy_from_slice(record);
    }
    let mut written = 0u32;
    let mut failed = 0u32;
    let mut off = 0u64;
    while off + bs as u64 <= g.size {
        if !part.is_bad(off).map_err(WriteError::Flash)? {
            let ok = (|| -> Result<bool, F::Error> {
                part.read(off, have)?;
                if have == want {
                    return Ok(true);
                }
                part.erase(off)?;
                part.write(off, &want[..used])?;
                part.read(off, have)?;
                Ok(have == want)
            })();
            match ok {
                Ok(true) => written += 1,
                _ => failed += 1,
            }
        }
        off += bs as u64;
    }
    if written == 0 {
        return Err(WriteError::NoBlockWritten);
    }
    // What will boot2 see?
    let after = scan(part).map_err(WriteError::Flash)?;
    if after.record.as_ref() != Some(record) || !after.consistent {
        return Err(WriteError::NotReadBack { failed, good: after.good_blocks });
    }
    Ok(failed)
}
